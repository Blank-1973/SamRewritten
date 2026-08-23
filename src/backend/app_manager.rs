// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Paul <abonnementspaul (at) gmail.com>
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, version 3.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

use crate::backend::connected_steam::ConnectedSteam;
use crate::backend::key_value::{KeyValue, KeyValueData};
use crate::backend::local_stats::schema_languages;
use crate::backend::stat_definitions::{
    AchievementDefinition, AchievementInfo, BaseStatDefinition, FloatStatDefinition, FloatStatInfo,
    IntStatInfo, IntegerStatDefinition, StatDefinition, StatInfo,
};
use crate::backend::stats_access::{AppScoped, StatsAccess, Stealth};
use crate::backend::types::UserStatType;
use crate::backend::user_unlock_times::{self, AchievementUnlock};
use crate::dev_println;
use crate::steam_client::steamworks_types::{AppId_t, CSteamID};
use crate::utils::ipc_types::SamError;
use crate::utils::steam_locator::SteamLocator;
use std::cell::RefCell;
use std::collections::HashSet;
use std::env;
use std::rc::Rc;
use std::time::UNIX_EPOCH;

pub struct AppManager {
    app_id: AppId_t,
    connected_steam: Rc<ConnectedSteam>,
    stats: Box<dyn StatsAccess>,
    /// Parsed language, so a change re-parses instead of serving stale names.
    loaded_language: Option<String>,
    user_stats_received: bool,
    /// A deferred or imported write does not reach a store until much later.
    pending_writes: RefCell<Vec<(String, bool)>>,
    achievement_definitions: Vec<AchievementDefinition>,
    stat_definitions: Vec<StatDefinition>,
}

pub struct StatState<T> {
    pub min: T,
    pub max: T,
    pub increment_only: bool,
    pub default: T,
    pub current: Option<T>,
}

#[cfg(any(debug_assertions, test))]
fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;

    for byte in data {
        a = (a + *byte as u32) % 65521;
        b = (b + a) % 65521;
    }

    (b << 16) | a
}

impl AppManager {
    pub fn new_connected(
        app_id: AppId_t,
        stealth: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        unsafe {
            env::remove_var("SteamGameId");
            env::remove_var("SteamOverlayGameId");
            if stealth {
                env::remove_var("SteamAppId");
            } else {
                env::set_var("SteamAppId", app_id.to_string());
            }
        }

        #[cfg(feature = "cli")]
        let silent = false;
        #[cfg(feature = "gui")]
        let silent = true;

        let connected_steam = match ConnectedSteam::new(silent) {
            Ok(c) => c,
            Err(e) => {
                return Err(e);
            }
        };

        if stealth {
            let client_user = connected_steam.client_user().inspect_err(|e| {
                eprintln!("[APP MANAGER] Could not open IClientUser to check ownership: {e}");
            })?;
            if !client_user.get_subscribed_apps().contains(&app_id) {
                eprintln!("[APP MANAGER] App {app_id} is not on this account");
                return Err(Box::new(SamError::SteamConnectionFailed));
            }
        }

        let connected_steam = Rc::new(connected_steam);
        let stats: Box<dyn StatsAccess> = if stealth {
            Box::new(Stealth::new(Rc::clone(&connected_steam), app_id)?)
        } else {
            Box::new(AppScoped::new(Rc::clone(&connected_steam)))
        };

        Ok(Self {
            app_id,
            connected_steam,
            stats,
            loaded_language: None,
            user_stats_received: false,
            pending_writes: RefCell::new(Vec::new()),
            achievement_definitions: vec![],
            stat_definitions: vec![],
        })
    }

    fn request_current_stats(&mut self) -> Result<(), SamError> {
        if self.user_stats_received {
            return Ok(());
        }

        // Offline (or backend unreachable): Steam never services the
        // UserStatsReceived callback, so waiting would just stall for the full
        // timeout. Skip it and fall back to the on-disk stats cache so the app
        // still loads. Treat a failed BLoggedOn check as "assume online".
        if self.connected_steam.user.b_logged_on() == Ok(false) {
            eprintln!(
                "[APP MANAGER] Steam is offline; loading from cached stats without a live request"
            );
            return Ok(());
        }

        if self.stats.prime()? {
            self.user_stats_received = true;
        }
        Ok(())
    }

    /// Resolve a `friend` string — either a SteamID64 or a persona name from the
    /// current user's friends list — then read their unlock times for this app.
    pub fn fetch_friend_unlock_times(
        &mut self,
        friend: &str,
    ) -> Result<Vec<AchievementUnlock>, SamError> {
        let friend = friend.trim();
        // A bare SteamID64 is used directly; anything else is a persona name
        // matched against the live friends list.
        let steam_id64 = match friend.parse::<u64>() {
            Ok(id) if id >= user_unlock_times::STEAMID64_BASE => id,
            _ => user_unlock_times::find_friend_steamid64(&self.connected_steam.friends, friend)
                .ok_or_else(|| {
                    eprintln!("[APP MANAGER] Friend '{friend}' not found in friends list");
                    SamError::UnknownError
                })?,
        };
        self.fetch_user_unlock_times(steam_id64)
    }

    /// Count a user's achieved vs total achievements for this app, reusing the
    /// same resolution `fetch_user_unlock_times` does (local cache or live API).
    /// Returns `(achieved, total)`; a private profile surfaces as `ProfilePrivate`.
    pub fn fetch_user_achievement_count(
        &mut self,
        steam_id64: u64,
    ) -> Result<(u32, u32), SamError> {
        let list = self.fetch_user_unlock_times(steam_id64)?;
        let achieved = list.iter().filter(|a| a.achieved).count() as u32;
        Ok((achieved, list.len() as u32))
    }

    /// Fetch another user's achievement unlock times for this app. Steam only
    /// writes an on-disk stats cache for accounts that have signed in on this
    /// machine, so locally-cached accounts get a single bulk parse while remote
    /// friends fall back to the per-user API (names from one bulk schema parse).
    pub fn fetch_user_unlock_times(
        &mut self,
        steam_id64: u64,
    ) -> Result<Vec<AchievementUnlock>, SamError> {
        let account_id = user_unlock_times::account_id(steam_id64);
        let steam_id = CSteamID {
            m_steamid: steam_id64,
        };

        // A locally-cached account (signed in on this machine) has its stats on
        // disk, so bulk-parse those directly — no live request, which also avoids
        // a spurious timeout masking data we already hold.
        let user_path = user_unlock_times::user_stats_file(account_id, self.app_id)?;
        if user_path.exists() {
            return user_unlock_times::read_unlock_times(account_id, self.app_id);
        }

        // No local cache: depend on the live request, so a non-OK result means
        // the target's game details / achievements are private.
        self.stats
            .request_other_user_stats(steam_id)
            .inspect_err(|e| {
                eprintln!("[APP MANAGER] Could not load stats for {steam_id64}: {e:?}");
            })?;

        let names = user_unlock_times::read_schema_achievements(self.app_id)?;
        let mut out = Vec::with_capacity(names.len());
        for (api_name, display_name) in names {
            let (achieved, unlock_time) = self
                .stats
                .get_other_user_achievement(steam_id, &api_name)
                .unwrap_or((false, 0));
            out.push(AchievementUnlock {
                api_name,
                display_name,
                achieved,
                unlock_time: if achieved && unlock_time > 0 {
                    Some(unlock_time)
                } else {
                    None
                },
            });
        }
        Ok(out)
    }

    fn ensure_definitions(&mut self, language: &str) -> Result<(), SamError> {
        if self.loaded_language.as_deref() != Some(language) {
            self.load_definitions(language)?;
        }
        Ok(())
    }

    /// Schemas disagree on how they spell a language (`LATAM` vs `latam`), so the
    /// global pick is matched loosely and answered with this schema's own spelling.
    /// A language this app lacks falls back to the game's, not to english.
    fn resolve_language(&self, language: &str, schema: &KeyValue) -> String {
        if let Some(offered) = self.schema_spelling(language, schema) {
            return offered;
        }
        // An override is stored in the user's own spelling, not the schema's.
        match self.stats.current_game_language() {
            Some(current) if !current.is_empty() => {
                self.schema_spelling(&current, schema).unwrap_or(current)
            }
            _ => "english".to_string(),
        }
    }

    fn schema_spelling(&self, language: &str, schema: &KeyValue) -> Option<String> {
        if language.is_empty() {
            return None;
        }
        schema_languages(schema)
            .into_iter()
            .find(|l| l.eq_ignore_ascii_case(language))
    }

    // Reference: https://github.com/gibbed/SteamAchievementManager/blob/master/SAM.Game/Manager.cs
    /// `language` is a Steam schema language name; empty means the game's own.
    pub fn load_definitions(&mut self, language: &str) -> Result<(), SamError> {
        self.request_current_stats()?;
        let steam_locator_lock = SteamLocator::global();
        let steam_locator = steam_locator_lock.read().unwrap();

        let bin_file = match steam_locator.get_user_game_stats_schema(&self.app_id) {
            Ok(bin_file) => bin_file,
            Err(e) => {
                eprintln!("[APP MANAGER] Error getting user game stats file: {}", e);
                return Err(e);
            }
        };

        #[cfg(debug_assertions)]
        {
            match std::fs::read(&bin_file) {
                Ok(bytes) => {
                    dev_println!(
                        "APPMAN",
                        "Loading user game stats file {} (Checksum: {:08x})",
                        bin_file.display(),
                        adler32(&bytes)
                    );
                }
                Err(e) => {
                    dev_println!("APPMAN", "Error loading user game stats file: {}", e);
                }
            };
        }

        let kv = match KeyValue::load_as_binary(&bin_file) {
            Ok(kv) => kv,
            Err(e) => {
                eprintln!(
                    "[APP MANAGER] Error loading key value from path {}: {:?}",
                    bin_file.display(),
                    e
                );
                return Err(SamError::UnknownError);
            }
        };

        let current_language = self.resolve_language(language, &kv);
        dev_println!(
            "APPMAN",
            "Reading schema in {current_language:?} (asked for {language:?})"
        );
        let stats = kv.get(&self.app_id.to_string());
        let stats = stats.get("stats");

        let mut stat_definitions: Vec<StatDefinition> = vec![];
        let mut achievement_definitions: Vec<AchievementDefinition> = vec![];

        for (_, stat) in stats.children.iter() {
            if !stat.valid {
                continue;
            }

            let mut type_ = UserStatType::Invalid;

            // Schema in the new format?
            let type_node = stat.get("type");
            if let KeyValueData::String(ref type_str) = type_node.data
                && let Ok(parsed) = type_str.parse::<UserStatType>()
            {
                type_ = parsed;
            }

            // Schema in the old format?
            if type_ == UserStatType::Invalid {
                let type_int_node = stat.get("type_int");

                let raw_type = if type_int_node.valid {
                    type_int_node.as_i32(0)
                } else {
                    type_node.as_i32(0)
                };

                type_ = UserStatType::try_from(raw_type as u8).unwrap_or(UserStatType::Invalid);
            }

            match type_ {
                UserStatType::Invalid => {
                    eprintln!("[APP MANAGER] Failed to parse user stat type: {type_node:?}");
                    continue;
                }

                UserStatType::Integer => {
                    let id = stat.get("name").as_string("");
                    let name = Self::get_localized_string(
                        stat.get("display").get("name"),
                        &current_language,
                        &id,
                    );
                    stat_definitions.push(StatDefinition::Integer(IntegerStatDefinition {
                        base: BaseStatDefinition {
                            id: stat.get("name").as_string(""),
                            display_name: name,
                            permission: stat.get("permission").as_i32(0),
                            app_id: self.app_id,
                        },
                        min_value: stat.get("min").as_i32(i32::MIN),
                        max_value: stat.get("max").as_i32(i32::MAX),
                        max_change: stat.get("maxchange").as_i32(0),
                        increment_only: stat.get("incrementonly").as_bool(false),
                        default_value: stat.get("default").as_i32(0),
                        set_by_trusted_game_server: stat.get("bSetByTrustedGS").as_bool(false),
                    }));
                }

                UserStatType::Float | UserStatType::AverageRate => {
                    let id = stat.get("name").as_string("");
                    let name = Self::get_localized_string(
                        stat.get("display").get("name"),
                        &current_language,
                        &id,
                    );
                    stat_definitions.push(StatDefinition::Float(FloatStatDefinition {
                        base: BaseStatDefinition {
                            id: stat.get("name").as_string(""),
                            display_name: name,
                            permission: stat.get("permission").as_i32(0),
                            app_id: self.app_id,
                        },
                        min_value: stat.get("min").as_f32(f32::MIN),
                        max_value: stat.get("max").as_f32(f32::MAX),
                        max_change: stat.get("maxchange").as_f32(0f32),
                        increment_only: stat.get("incrementonly").as_bool(false),
                        default_value: stat.get("default").as_f32(0f32),
                    }));
                }

                UserStatType::Achievements | UserStatType::GroupAchievements => {
                    for bits in stat.children.iter() {
                        if bits.0.to_lowercase() != "bits" {
                            continue;
                        }

                        if !bits.1.valid || bits.1.children.is_empty() {
                            dev_println!("APPMAN", "Invalid achievements bits.1: {:?}", bits.1);
                            continue;
                        }

                        for bit in bits.1.children.iter() {
                            let id = bit.1.get("name").as_string("");
                            let name = Self::get_localized_string(
                                bit.1.get("display").get("name"),
                                &current_language,
                                &id,
                            );
                            let description = Self::get_localized_string(
                                bit.1.get("display").get("desc"),
                                &current_language,
                                "",
                            );

                            achievement_definitions.push(AchievementDefinition {
                                id,
                                app_id: self.app_id,
                                name,
                                description,
                                icon_normal: format!("https://cdn.steamstatic.com/steamcommunity/public/images/apps/{}/{}", self.app_id, bit.1.get("display").get("icon").as_string("")),
                                icon_locked: format!("https://cdn.steamstatic.com/steamcommunity/public/images/apps/{}/{}", self.app_id, bit.1.get("display").get("icon_gray").as_string("")),
                                is_hidden: bit.1.get("display").get("hidden").as_bool(false),
                                permission: bit.1.get("permission").as_i32(0),
                            })
                        }
                    }
                }
            }
        }

        self.stat_definitions = stat_definitions;
        self.achievement_definitions = achievement_definitions;
        self.loaded_language = Some(language.to_owned());

        Ok(())
    }

    // Reference: https://github.com/gibbed/SteamAchievementManager/blob/master/SAM.Game/Manager.cs#L420
    pub fn get_achievements(
        &mut self,
        with_global_achieved: bool,
        language: &str,
    ) -> Result<Vec<AchievementInfo>, SamError> {
        self.ensure_definitions(language)?;

        let global_stats_fetched =
            with_global_achieved && self.stats.request_global_percentages()?;

        let mut achievement_infos: Vec<AchievementInfo> = vec![];

        for def in self.achievement_definitions.iter() {
            if def.id.is_empty() {
                dev_println!("APPMAN", "Achievement definition ID is empty:");
                dev_println!("{def:?}");
                continue;
            }

            let def_id = &def.id;
            match self.stats.get_achievement_and_unlock_time(def_id) {
                Ok((is_achieved, unlock_time)) => {
                    let global_achieved_percent = if !global_stats_fetched {
                        None
                    } else {
                        match self.stats.get_achievement_achieved_percent(def_id) {
                            Ok(percent) => Some(percent),
                            Err(_) => {
                                dev_println!(
                                    "APPSRV",
                                    "Failed to get achievement percent for achievement: {def_id}"
                                );
                                None
                            }
                        }
                    };

                    achievement_infos.push(AchievementInfo {
                        id: def_id.to_string(),
                        is_achieved,
                        unlock_time: if is_achieved && unlock_time > 0 {
                            UNIX_EPOCH
                                .checked_add(std::time::Duration::from_secs(unlock_time as u64))
                        } else {
                            None
                        },
                        icon_normal: def.icon_normal.clone(),
                        icon_locked: if def.icon_locked.is_empty() {
                            def.icon_normal.clone()
                        } else {
                            def.icon_locked.clone()
                        },
                        permission: def.permission,
                        name: def.name.clone(),
                        description: def.description.clone(),
                        global_achieved_percent,
                    });
                }
                Err(_) => {
                    dev_println!(
                        "APPSRV",
                        "Failed to get achievement info for achievement: {def_id}"
                    );
                    continue;
                }
            }
        }

        dev_println!(
            "APPMAN",
            "Loaded {} achievement definitions for {} achievements for app {}",
            self.achievement_definitions.len(),
            achievement_infos.len(),
            self.app_id
        );

        let readable = self
            .achievement_definitions
            .iter()
            .filter(|def| !def.id.is_empty())
            .count();
        if achievement_infos.is_empty() && readable > 0 {
            eprintln!(
                "[APP MANAGER] Steam served none of the {readable} achievements for app {}",
                self.app_id
            );
            return Err(SamError::SteamConnectionFailed);
        }

        Ok(achievement_infos)
    }

    // Reference: https://github.com/gibbed/SteamAchievementManager/blob/master/SAM.Game/Manager.cs#L519
    pub fn get_statistics(&mut self, language: &str) -> Result<Vec<StatInfo>, SamError> {
        let mut statistics_info: Vec<StatInfo> = vec![];

        self.ensure_definitions(language)?;

        for stat in self.stat_definitions.iter() {
            match stat {
                StatDefinition::Float(definition) => {
                    if definition.base.id.is_empty() {
                        continue;
                    }

                    let stat_value = match self.stats.get_stat_float(&definition.base.id) {
                        Ok(value) => {
                            if value.is_nan() {
                                dev_println!(
                                    "APPMAN",
                                    "Converting NAN stat float value to 0: {}",
                                    &definition.base.id
                                );
                                0f32
                            } else {
                                value
                            }
                        }
                        Err(_) => {
                            let stat_id = definition.base.id.to_string();
                            dev_println!(
                                "APPSRV",
                                "Failed to get float stat info for stat: {stat_id}"
                            );
                            continue;
                        }
                    };

                    statistics_info.push(StatInfo::Float(FloatStatInfo {
                        id: definition.base.id.clone(),
                        app_id: definition.base.app_id,
                        display_name: definition.base.display_name.clone(),
                        float_value: stat_value,
                        original_value: stat_value,
                        is_increment_only: definition.increment_only,
                        permission: definition.base.permission,
                        min_value: definition.min_value,
                        max_value: definition.max_value,
                    }));
                }

                StatDefinition::Integer(definition) => {
                    if definition.base.id.is_empty() {
                        continue;
                    }

                    let stat_value = match self.stats.get_stat_i32(&definition.base.id) {
                        Ok(value) => value,
                        Err(_) => {
                            let stat_id = definition.base.id.to_string();
                            dev_println!(
                                "APPSRV",
                                "Failed to get int stat info for stat: {stat_id}"
                            );
                            continue;
                        }
                    };

                    statistics_info.push(StatInfo::Integer(IntStatInfo {
                        id: definition.base.id.clone(),
                        app_id: definition.base.app_id,
                        display_name: definition.base.display_name.clone(),
                        int_value: stat_value,
                        original_value: stat_value,
                        is_increment_only: definition.increment_only,
                        permission: definition.base.permission,
                        min_value: definition.min_value,
                        max_value: definition.max_value,
                    }));
                }
            };
        }

        let readable = self
            .stat_definitions
            .iter()
            .filter(|d| match d {
                StatDefinition::Float(def) => !def.base.id.is_empty(),
                StatDefinition::Integer(def) => !def.base.id.is_empty(),
            })
            .count();
        let has_achievements = self
            .achievement_definitions
            .iter()
            .any(|def| !def.id.is_empty());
        if statistics_info.is_empty() && readable > 0 && !has_achievements {
            eprintln!(
                "[APP MANAGER] Steam served none of the {readable} stats for app {}",
                self.app_id
            );
            return Err(SamError::SteamConnectionFailed);
        }

        Ok(statistics_info)
    }

    pub fn set_achievement(
        &self,
        achievement_id: &str,
        unlock: bool,
        store: bool,
    ) -> Result<bool, SamError> {
        let written = if unlock {
            self.stats.set_achievement(achievement_id)
        } else {
            self.stats.clear_achievement(achievement_id)
        };
        if written.is_err() {
            return Err(SamError::LockUnlockAchievementFailed);
        }

        self.pending_writes
            .borrow_mut()
            .push((achievement_id.to_string(), unlock));

        if !store {
            return Ok(true);
        }

        self.store_stats_and_achievements()
    }

    /// `Ok(false)` is Steam accepting the call and declining to store. Nothing
    /// set before this point is committed until it returns true, so callers
    /// file history entries and report success on the answer.
    pub fn store_stats_and_achievements(&self) -> Result<bool, SamError> {
        // Taken before the store, so a failed one leaves nothing behind.
        let mut pending = std::mem::take(&mut *self.pending_writes.borrow_mut());
        pending.reverse();
        let mut seen = HashSet::new();
        pending.retain(|(id, _)| seen.insert(id.clone()));

        let stored = self
            .stats
            .store_stats()
            .map_err(|_| SamError::StatStoreFailed)?;

        if stored && !pending.is_empty() {
            let failed = pending
                .iter()
                .filter(|(id, expected)| !self.stats.verify_achievement(id, *expected))
                .count();
            if failed == pending.len() {
                eprintln!(
                    "[APP MANAGER] no achievement write took for app {}; refusing to report success",
                    self.app_id
                );
                return Err(SamError::LockUnlockAchievementFailed);
            }
            if failed > 0 {
                eprintln!(
                    "[APP MANAGER] {failed} of {} achievement writes did not take for app {}",
                    pending.len(),
                    self.app_id
                );
            }
        }
        if !stored {
            eprintln!(
                "[APP MANAGER] Steam declined to store for app {}",
                self.app_id
            );
        }
        Ok(stored)
    }

    pub fn read_int_stat_state(&self, id: &str) -> StatState<i32> {
        let (min, max, increment_only, default) = self
            .stat_definitions
            .iter()
            .find_map(|d| match d {
                StatDefinition::Integer(def) if def.base.id == id => Some(def),
                _ => None,
            })
            .map(|d| (d.min_value, d.max_value, d.increment_only, d.default_value))
            .unwrap_or((i32::MIN, i32::MAX, false, 0));
        let current = self.stats.get_stat_i32(id).ok();
        StatState {
            min,
            max,
            increment_only,
            default,
            current,
        }
    }

    pub fn read_float_stat_state(&self, id: &str) -> StatState<f32> {
        let (min, max, increment_only, default) = self
            .stat_definitions
            .iter()
            .find_map(|d| match d {
                StatDefinition::Float(def) if def.base.id == id => Some(def),
                _ => None,
            })
            .map(|d| (d.min_value, d.max_value, d.increment_only, d.default_value))
            .unwrap_or((f32::MIN, f32::MAX, false, 0.0));
        let current = self.stats.get_stat_float(id).ok();
        StatState {
            min,
            max,
            increment_only,
            default,
            current,
        }
    }

    pub fn unlock_all_achievements(&mut self) -> Result<bool, SamError> {
        // Only ids and flags are used here, so reuse whatever is already parsed
        // rather than forcing a re-parse in another language.
        let language = self.loaded_language.clone().unwrap_or_default();
        let achievements = self.get_achievements(false, &language)?;
        let mut has_failures = false;
        for achievement in achievements {
            if achievement.is_achieved {
                continue;
            }

            if achievement.permission != 0 {
                continue;
            }

            if self.stats.set_achievement(achievement.id.as_str()).is_err() {
                eprintln!(
                    "[APP MANAGER] Failed to unlock achievement for app {} while unlocking all: {achievement:?}",
                    self.app_id
                );
                has_failures = true;
                continue;
            }
            self.pending_writes
                .borrow_mut()
                .push((achievement.id, true));
        }

        let stored = self.store_stats_and_achievements()?;

        if has_failures {
            Err(SamError::LockUnlockAchievementFailed)
        } else {
            Ok(stored)
        }
    }

    pub fn set_stat_i32(&self, stat_name: &str, stat_value: i32) -> Result<bool, SamError> {
        match self.stats.set_stat_i32(stat_name, stat_value) {
            Ok(_) => self.store_stats_and_achievements(),
            Err(_) => Err(SamError::UnknownError),
        }
    }

    pub fn set_stat_f32(&self, stat_name: &str, stat_value: f32) -> Result<bool, SamError> {
        match self.stats.set_stat_float(stat_name, stat_value) {
            Ok(_) => self.store_stats_and_achievements(),
            Err(_) => Err(SamError::UnknownError),
        }
    }

    pub fn reset_all_stats(&self, achievements_too: bool) -> Result<bool, SamError> {
        if achievements_too {
            self.pending_writes.borrow_mut().clear();
        }
        match self.stats.reset_all_stats(achievements_too) {
            Ok(true) => self.store_stats_and_achievements(),
            Ok(false) => {
                eprintln!(
                    "[APP MANAGER] Steam refused ResetAllStats for app {}",
                    self.app_id
                );
                Ok(false)
            }
            Err(e) => {
                eprintln!(
                    "[APP MANAGER] ResetAllStats failed for app {}: {e:?}",
                    self.app_id
                );
                Err(SamError::UnknownError)
            }
        }
    }

    fn get_localized_string(kv: &KeyValue, language: &str, default_value: &str) -> String {
        let name = kv.get(language).as_string("");
        if !name.is_empty() {
            return name;
        }

        if language != "english" {
            let name = kv.get("english").as_string("");
            if !name.is_empty() {
                return name;
            }
        }

        let name = kv.as_string("");
        if !name.is_empty() {
            return name;
        }

        default_value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use crate::backend::app_manager::adler32;

    #[test]
    fn test_adler32() {
        println!("Adler null: {:08x}", adler32(&[]));
    }
}
