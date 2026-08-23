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

//! Exporting `SteamAppId` is what registers the process as *running* the app,
//! which is what makes the user appear in-game. `Stealth` never sets it and
//! drives `IClientUserStats` instead, passing the app as a `CGameID` per call.

use crate::backend::connected_steam::ConnectedSteam;
use crate::backend::local_config;
use crate::backend::user_unlock_times;
use crate::dev_println;
use crate::steam_client::client_user_stats_map_wrapper::ClientUserStatsMap;
use crate::steam_client::steamworks_types::{
    AppId_t, CSteamID, EResult, GlobalAchievementPercentagesReady_t, SteamAPICall_t,
    UserStatsReceived_t,
};
use crate::steam_client::wrapper_types::{SteamCallbackId, SteamClientError};
use crate::utils::app_paths::get_executable_path;
use crate::utils::ipc_types::SamError;
use crate::utils::steam_locator::SteamLocator;
use std::cell::{Cell, RefCell};
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

const FRAME: std::time::Duration = std::time::Duration::from_millis(17);
/// 30 s at ~60 fps; a bulk fan-out queues many requests through Steam's one IPC.
const USER_STATS_FRAMES: u32 = 1800;
const PERCENTAGES_FRAMES: u32 = 600;

static STEALTH: AtomicBool = AtomicBool::new(false);

pub fn set_stealth(on: bool) {
    STEALTH.store(on, Ordering::Relaxed);
}

pub fn stealth() -> bool {
    STEALTH.load(Ordering::Relaxed)
}

/// The one place an app server is spawned from, so no call site forgets the mode.
pub fn app_server_command(app_id: AppId_t) -> Command {
    spawn_app_server(app_id, stealth())
}

pub fn idle_app_server_command(app_id: AppId_t) -> Command {
    spawn_app_server(app_id, false)
}

fn spawn_app_server(app_id: AppId_t, stealth: bool) -> Command {
    let mut command = Command::new(get_executable_path());
    command.arg(format!("--app={app_id}"));
    if stealth {
        command.arg("--stealth");
    }
    command
}

pub trait StatsAccess {
    fn prime(&self) -> Result<bool, SamError>;

    fn get_achievement_and_unlock_time(
        &self,
        achievement_name: &str,
    ) -> Result<(bool, u32), SteamClientError>;
    fn set_achievement(&self, achievement_name: &str) -> Result<(), SteamClientError>;
    fn clear_achievement(&self, achievement_name: &str) -> Result<(), SteamClientError>;

    fn get_stat_i32(&self, stat_name: &str) -> Result<i32, SteamClientError>;
    fn get_stat_float(&self, stat_name: &str) -> Result<f32, SteamClientError>;
    fn set_stat_i32(&self, stat_name: &str, value: i32) -> Result<i32, SteamClientError>;
    fn set_stat_float(&self, stat_name: &str, value: f32) -> Result<f32, SteamClientError>;

    fn store_stats(&self) -> Result<bool, SteamClientError>;
    fn reset_all_stats(&self, achievements_too: bool) -> Result<bool, SteamClientError>;

    fn request_global_percentages(&self) -> Result<bool, SamError>;
    fn get_achievement_achieved_percent(
        &self,
        achievement_name: &str,
    ) -> Result<f32, SteamClientError>;

    fn request_other_user_stats(&self, steam_id: CSteamID) -> Result<(), SamError>;

    fn get_other_user_achievement(
        &self,
        steam_id: CSteamID,
        achievement_name: &str,
    ) -> Option<(bool, u32)>;

    fn current_game_language(&self) -> Option<String>;

    /// `SetAchievement` (slot 13) is the one call with no `steamui` call site to
    /// re-derive it from, so read back through slot 12 to catch a vtable shift.
    fn verify_achievement(&self, achievement_name: &str, expected: bool) -> bool {
        match self.get_achievement_and_unlock_time(achievement_name) {
            Ok((achieved, _)) => achieved == expected,
            Err(_) => true,
        }
    }
}

pub struct AppScoped {
    steam: Rc<ConnectedSteam>,
}

impl AppScoped {
    pub fn new(steam: Rc<ConnectedSteam>) -> Self {
        Self { steam }
    }
}

impl StatsAccess for AppScoped {
    fn prime(&self) -> Result<bool, SamError> {
        let steam_id = self.steam.user.get_steam_id().map_err(|e| {
            eprintln!("[APP MANAGER] Error getting steam id: {e}");
            SamError::UnknownError
        })?;
        dev_println!(
            "APPSRV",
            "Requesting current stats for current user: {steam_id:?}"
        );
        match wait_for_user_stats(&self.steam, steam_id) {
            Ok(code) if code == EResult::k_EResultOK as i32 => Ok(true),
            Ok(code) => {
                eprintln!(
                    "[APP MANAGER] RequestCurrentStats returned {code}; continuing with cached stats"
                );
                Ok(false)
            }
            Err(SamError::Timeout) => {
                eprintln!(
                    "[APP MANAGER] RequestCurrentStats timed out; continuing with cached stats"
                );
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    fn get_achievement_and_unlock_time(
        &self,
        achievement_name: &str,
    ) -> Result<(bool, u32), SteamClientError> {
        self.steam
            .user_stats
            .get_achievement_and_unlock_time(achievement_name)
    }

    fn set_achievement(&self, achievement_name: &str) -> Result<(), SteamClientError> {
        self.steam.user_stats.set_achievement(achievement_name)
    }

    fn clear_achievement(&self, achievement_name: &str) -> Result<(), SteamClientError> {
        self.steam.user_stats.clear_achievement(achievement_name)
    }

    fn get_stat_i32(&self, stat_name: &str) -> Result<i32, SteamClientError> {
        self.steam.user_stats.get_stat_i32(stat_name)
    }

    fn get_stat_float(&self, stat_name: &str) -> Result<f32, SteamClientError> {
        self.steam.user_stats.get_stat_float(stat_name)
    }

    fn set_stat_i32(&self, stat_name: &str, value: i32) -> Result<i32, SteamClientError> {
        self.steam.user_stats.set_stat_i32(stat_name, value)
    }

    fn set_stat_float(&self, stat_name: &str, value: f32) -> Result<f32, SteamClientError> {
        self.steam.user_stats.set_stat_float(stat_name, value)
    }

    fn store_stats(&self) -> Result<bool, SteamClientError> {
        self.steam.user_stats.store_stats()
    }

    fn reset_all_stats(&self, achievements_too: bool) -> Result<bool, SteamClientError> {
        self.steam.user_stats.reset_all_stats(achievements_too)
    }

    fn request_global_percentages(&self) -> Result<bool, SamError> {
        let handle = self
            .steam
            .user_stats
            .request_global_achievement_percentages()
            .map_err(|_| SamError::UnknownError)?;

        let ready = wait_for_call_result::<GlobalAchievementPercentagesReady_t>(
            &self.steam,
            handle,
            SteamCallbackId::GlobalAchievementPercentagesReady,
            PERCENTAGES_FRAMES,
        )?;
        let Some(ready) = ready else {
            eprintln!("[APP MANAGER] RequestGlobalAchievementPercentages timed out");
            return Ok(false);
        };

        let (game_id, code) = (ready.m_nGameID, ready.m_eResult);
        dev_println!(
            "APPSRV",
            "Global achievement percentages callback: game {game_id}, result {code}"
        );
        Ok(code == EResult::k_EResultOK as i32)
    }

    fn get_achievement_achieved_percent(
        &self,
        achievement_name: &str,
    ) -> Result<f32, SteamClientError> {
        self.steam
            .user_stats
            .get_achievement_achieved_percent(achievement_name)
    }

    fn request_other_user_stats(&self, steam_id: CSteamID) -> Result<(), SamError> {
        match wait_for_user_stats(&self.steam, steam_id)? {
            code if code == EResult::k_EResultOK as i32 => Ok(()),
            code => {
                eprintln!("[APP MANAGER] RequestUserStats returned {code}");
                Err(SamError::ProfilePrivate)
            }
        }
    }

    fn get_other_user_achievement(
        &self,
        steam_id: CSteamID,
        achievement_name: &str,
    ) -> Option<(bool, u32)> {
        self.steam
            .user_stats
            .get_user_achievement_and_unlock_time(steam_id, achievement_name)
            .ok()
    }

    fn current_game_language(&self) -> Option<String> {
        Some(self.steam.apps.get_current_game_language())
    }

    fn verify_achievement(&self, _achievement_name: &str, _expected: bool) -> bool {
        true
    }
}

pub struct Stealth {
    steam: Rc<ConnectedSteam>,
    map: ClientUserStatsMap,
    app_id: AppId_t,
    loaded: Cell<bool>,
    load_attempted: Cell<bool>,
    refusals: Cell<u8>,
    language: RefCell<Option<String>>,
}

impl Stealth {
    pub fn new(steam: Rc<ConnectedSteam>, app_id: AppId_t) -> Result<Self, SamError> {
        let map = steam.client_user_stats_map().map_err(|e| {
            eprintln!("[STEALTH] Could not open IClientUserStats: {e}");
            SamError::SteamConnectionFailed
        })?;
        Ok(Self {
            steam,
            map,
            app_id,
            loaded: Cell::new(false),
            load_attempted: Cell::new(false),
            refusals: Cell::new(0),
            language: RefCell::new(None),
        })
    }

    /// Steam silently declines every write for a game whose stats it has not
    /// loaded, and a bulk fan-out writes without reading first. Offline, skip.
    fn ensure_loaded(&self) {
        if self.loaded.get() || self.steam.user.b_logged_on() == Ok(false) {
            return;
        }
        let _ = self.prime();
    }

    fn forget_load_if_stale(&self, stored: &Result<bool, SteamClientError>) {
        if matches!(stored, Ok(false)) && self.loaded.replace(false) {
            self.load_attempted.set(false);
            self.refusals.set(0);
            eprintln!(
                "[STEALTH] App {} was loaded but refused a store; will reload",
                self.app_id
            );
        }
    }
}

impl StatsAccess for Stealth {
    fn prime(&self) -> Result<bool, SamError> {
        if self.loaded.get() {
            return Ok(true);
        }
        const MAX_REFUSALS: u8 = 3;
        if self.load_attempted.get() {
            return Ok(false);
        }

        match self.map.request_current_stats(self.app_id) {
            Ok(true) => {
                self.load_attempted.set(true);
                self.refusals.set(0);
            }
            Ok(false) => {
                self.refusals.set(self.refusals.get().saturating_add(1));
                if self.refusals.get() >= MAX_REFUSALS {
                    self.load_attempted.set(true);
                }
                eprintln!(
                    "[STEALTH] RequestCurrentStats refused for app {}",
                    self.app_id
                );
                return Ok(false);
            }
            Err(e) => {
                self.load_attempted.set(true);
                eprintln!(
                    "[STEALTH] RequestCurrentStats failed for app {}: {e}",
                    self.app_id
                );
                return Ok(false);
            }
        }

        for _ in 0..USER_STATS_FRAMES {
            self.map.run_engine_frame();
            for (app_id, result) in self.map.drain_user_stats_callbacks() {
                if app_id != self.app_id {
                    continue;
                }
                dev_println!(
                    "STLTH",
                    "User stats received: app {app_id}, result {result}"
                );
                self.loaded.set(result == EResult::k_EResultOK as i32);
                return Ok(self.loaded.get());
            }
            std::thread::sleep(FRAME);
        }

        eprintln!(
            "[STEALTH] RequestCurrentStats timed out for app {}",
            self.app_id
        );
        Ok(false)
    }

    fn get_achievement_and_unlock_time(
        &self,
        achievement_name: &str,
    ) -> Result<(bool, u32), SteamClientError> {
        self.map
            .get_achievement_and_unlock_time(self.app_id, achievement_name)
    }

    fn set_achievement(&self, achievement_name: &str) -> Result<(), SteamClientError> {
        self.ensure_loaded();
        self.map.set_achievement(self.app_id, achievement_name)
    }

    fn clear_achievement(&self, achievement_name: &str) -> Result<(), SteamClientError> {
        self.ensure_loaded();
        self.map.clear_achievement(self.app_id, achievement_name)
    }

    fn get_stat_i32(&self, stat_name: &str) -> Result<i32, SteamClientError> {
        self.map.get_stat_i32(self.app_id, stat_name)
    }

    fn get_stat_float(&self, stat_name: &str) -> Result<f32, SteamClientError> {
        self.map.get_stat_f32(self.app_id, stat_name)
    }

    fn set_stat_i32(&self, stat_name: &str, value: i32) -> Result<i32, SteamClientError> {
        self.ensure_loaded();
        self.map.set_stat_i32(self.app_id, stat_name, value)
    }

    fn set_stat_float(&self, stat_name: &str, value: f32) -> Result<f32, SteamClientError> {
        self.ensure_loaded();
        self.map.set_stat_f32(self.app_id, stat_name, value)
    }

    fn store_stats(&self) -> Result<bool, SteamClientError> {
        self.ensure_loaded();
        let stored = self.map.store_stats(self.app_id);
        self.forget_load_if_stale(&stored);
        stored
    }

    fn reset_all_stats(&self, achievements_too: bool) -> Result<bool, SteamClientError> {
        self.ensure_loaded();
        let reset = self.map.reset_all_stats(self.app_id, achievements_too);
        self.forget_load_if_stale(&reset);
        reset
    }

    fn request_global_percentages(&self) -> Result<bool, SamError> {
        let handle = self
            .map
            .request_global_achievement_percentages(self.app_id)
            .inspect_err(|e| eprintln!("[STEALTH] Could not reach IClientUserStats: {e}"))
            .unwrap_or(0);
        if handle == 0 {
            eprintln!(
                "[STEALTH] RequestGlobalAchievementPercentages refused for app {}",
                self.app_id
            );
            return Ok(false);
        }

        let Some(ready) = self
            .map
            .wait_for_api_call::<GlobalAchievementPercentagesReady_t>(
                handle,
                SteamCallbackId::GlobalAchievementPercentagesReady,
                PERCENTAGES_FRAMES,
            )
        else {
            eprintln!(
                "[STEALTH] RequestGlobalAchievementPercentages timed out for app {}",
                self.app_id
            );
            return Ok(false);
        };

        let (game_id, code) = (ready.m_nGameID, ready.m_eResult);
        dev_println!(
            "STLTH",
            "Global achievement percentages: game {game_id}, result {code}"
        );
        Ok(code == EResult::k_EResultOK as i32)
    }

    fn get_achievement_achieved_percent(
        &self,
        achievement_name: &str,
    ) -> Result<f32, SteamClientError> {
        self.map
            .get_achievement_achieved_percent(self.app_id, achievement_name)
    }

    fn request_other_user_stats(&self, steam_id: CSteamID) -> Result<(), SamError> {
        let handle = self
            .map
            .request_user_stats(steam_id, self.app_id)
            .map_err(|e| {
                eprintln!("[STEALTH] Error requesting user stats: {e}");
                SamError::UnknownError
            })?;

        let Some(received) = self.map.wait_for_api_call::<UserStatsReceived_t>(
            handle,
            SteamCallbackId::UserStatsReceived,
            USER_STATS_FRAMES,
        ) else {
            eprintln!("[STEALTH] Requesting user stats timed out");
            return Err(SamError::Timeout);
        };

        let (game_id, code, user) = (
            received.m_nGameID,
            received.m_eResult,
            received.m_steamIDUser,
        );
        dev_println!(
            "STEALTH",
            "User stats received: game {game_id}, result {code}, user {}",
            user.m_steamid
        );
        if code == EResult::k_EResultOK as i32 {
            Ok(())
        } else {
            eprintln!("[STEALTH] RequestUserStats returned {code}");
            Err(SamError::ProfilePrivate)
        }
    }

    fn get_other_user_achievement(
        &self,
        steam_id: CSteamID,
        achievement_name: &str,
    ) -> Option<(bool, u32)> {
        self.map
            .get_user_achievement_and_unlock_time(steam_id, self.app_id, achievement_name)
            .ok()
    }

    fn current_game_language(&self) -> Option<String> {
        if let Some(cached) = self.language.borrow().as_ref() {
            return Some(cached.clone());
        }
        let steam_id = self.steam.user.get_steam_id().ok()?;
        let account_id = user_unlock_times::account_id(steam_id.m_steamid);
        let override_language = SteamLocator::get_local_config_path(account_id)
            .and_then(|path| local_config::app_language(&path, self.app_id));
        let language =
            override_language.unwrap_or_else(|| self.steam.apps.get_current_game_language());
        *self.language.borrow_mut() = Some(language.clone());
        Some(language)
    }
}

fn wait_for_call_result<T>(
    steam: &ConnectedSteam,
    handle: SteamAPICall_t,
    expected: SteamCallbackId,
    frames: u32,
) -> Result<Option<T>, SamError> {
    for _ in 0..frames {
        let completed = steam.utils.is_api_call_completed(handle).map_err(|e| {
            eprintln!("[APP MANAGER] Error checking api call completed: {e}");
            SamError::UnknownError
        })?;

        if completed {
            let result = steam
                .utils
                .get_api_call_result::<T>(handle, expected)
                .map_err(|e| {
                    eprintln!("[APP MANAGER] Error getting api call result: {e}");
                    SamError::UnknownError
                })?;
            return Ok(Some(result));
        }

        std::thread::sleep(FRAME);
    }
    Ok(None)
}

/// App-scoped only: `ISteamUserStats` needs the process bound to the app.
fn wait_for_user_stats(steam: &ConnectedSteam, steam_id: CSteamID) -> Result<i32, SamError> {
    let handle = steam.user_stats.request_user_stats(steam_id).map_err(|e| {
        eprintln!("[APP MANAGER] Error requesting user stats: {e}");
        SamError::UnknownError
    })?;

    let received = wait_for_call_result::<UserStatsReceived_t>(
        steam,
        handle,
        SteamCallbackId::UserStatsReceived,
        USER_STATS_FRAMES,
    )?;
    let Some(result) = received else {
        eprintln!("[APP MANAGER] Requesting user stats timed out");
        return Err(SamError::Timeout);
    };

    let (game_id, code, user) = (result.m_nGameID, result.m_eResult, result.m_steamIDUser);
    dev_println!(
        "APPSRV",
        "User stats received callback: game {game_id}, result {code}, user {}",
        user.m_steamid
    );
    Ok(code)
}
