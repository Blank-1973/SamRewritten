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

// Per-app dispatcher for IClientUserStats. Every method takes a
// `CGameID const&` (a pointer in System V x86_64), which is how one
// instance serves any app id without the process being registered as
// running it.
//
// Slot names are ground truth: each stub in steamclient.so serialises its own
// method name for the IPC dispatch. Not settled by the names: which half of
// an overloaded pair is which. `*const c_void` is never called.

use crate::steam_client::steamworks_types::{CGameID, CSteamID, SteamAPICall_t};
use std::os::raw::{c_char, c_void};

#[repr(C)]
pub struct IClientUserStatsMap {
    pub vtable: *const IClientUserStatsMapVTable,
}

#[repr(C)]
pub struct IClientUserStatsMapVTable {
    pub get_num_stats: unsafe extern "C" fn(*mut IClientUserStatsMap, *const CGameID) -> u32,
    pub get_stat_name: *const c_void,
    pub get_stat_type: *const c_void,
    pub get_num_achievements: unsafe extern "C" fn(*mut IClientUserStatsMap, *const CGameID) -> u32,
    pub get_achievement_name: *const c_void,
    pub request_current_stats:
        unsafe extern "C" fn(*mut IClientUserStatsMap, *const CGameID) -> bool,
    pub deprecated_public_request_current_stats: *const c_void,
    // MSVC reverses overload order, so these two pairs swap on Windows.
    #[cfg(target_os = "windows")]
    pub get_stat_f32: unsafe extern "C" fn(
        *mut IClientUserStatsMap,
        *const CGameID,
        *const c_char,
        *mut f32,
    ) -> bool,
    #[cfg(target_os = "windows")]
    pub get_stat_i32: unsafe extern "C" fn(
        *mut IClientUserStatsMap,
        *const CGameID,
        *const c_char,
        *mut i32,
    ) -> bool,
    #[cfg(not(target_os = "windows"))]
    pub get_stat_i32: unsafe extern "C" fn(
        *mut IClientUserStatsMap,
        *const CGameID,
        *const c_char,
        *mut i32,
    ) -> bool,
    #[cfg(not(target_os = "windows"))]
    pub get_stat_f32: unsafe extern "C" fn(
        *mut IClientUserStatsMap,
        *const CGameID,
        *const c_char,
        *mut f32,
    ) -> bool,
    #[cfg(target_os = "windows")]
    pub set_stat_f32:
        unsafe extern "C" fn(*mut IClientUserStatsMap, *const CGameID, *const c_char, f32) -> bool,
    #[cfg(target_os = "windows")]
    pub set_stat_i32:
        unsafe extern "C" fn(*mut IClientUserStatsMap, *const CGameID, *const c_char, i32) -> bool,
    #[cfg(not(target_os = "windows"))]
    pub set_stat_i32:
        unsafe extern "C" fn(*mut IClientUserStatsMap, *const CGameID, *const c_char, i32) -> bool,
    #[cfg(not(target_os = "windows"))]
    pub set_stat_f32:
        unsafe extern "C" fn(*mut IClientUserStatsMap, *const CGameID, *const c_char, f32) -> bool,
    pub update_avg_rate_stat: *const c_void,
    pub get_achievement: unsafe extern "C" fn(
        *mut IClientUserStatsMap,
        *const CGameID,
        *const c_char,
        *mut bool,
        *mut u32,
    ) -> bool,
    pub set_achievement:
        unsafe extern "C" fn(*mut IClientUserStatsMap, *const CGameID, *const c_char) -> bool,
    pub clear_achievement:
        unsafe extern "C" fn(*mut IClientUserStatsMap, *const CGameID, *const c_char) -> bool,
    pub get_achievement_progress: *const c_void,
    pub store_stats: unsafe extern "C" fn(*mut IClientUserStatsMap, *const CGameID) -> bool,
    pub get_achievement_icon: *const c_void,
    pub bget_achievement_icon_url: *const c_void,
    pub get_achievement_display_attribute: *const c_void,
    pub indicate_achievement_progress: *const c_void,
    pub set_max_stats_loaded: *const c_void,
    // Slots 22-26 take the user id by value and the game id by reference.
    pub request_user_stats:
        unsafe extern "C" fn(*mut IClientUserStatsMap, CSteamID, *const CGameID) -> SteamAPICall_t,
    pub get_user_stat_i32: *const c_void,
    pub get_user_stat_f32: *const c_void,
    pub get_user_achievement: unsafe extern "C" fn(
        *mut IClientUserStatsMap,
        CSteamID,
        *const CGameID,
        *const c_char,
        *mut bool,
        *mut u32,
    ) -> bool,
    pub get_user_achievement_progress: *const c_void,
    pub reset_all_stats:
        unsafe extern "C" fn(*mut IClientUserStatsMap, *const CGameID, bool) -> bool,
    pub find_or_create_leaderboard: *const c_void,
    pub find_leaderboard: *const c_void,
    pub get_leaderboard_name: *const c_void,
    pub get_leaderboard_entry_count: *const c_void,
    pub get_leaderboard_sort_method: *const c_void,
    pub get_leaderboard_display_type: *const c_void,
    pub download_leaderboard_entries: *const c_void,
    pub download_leaderboard_entries_for_users: *const c_void,
    pub get_downloaded_leaderboard_entry: *const c_void,
    pub attach_leaderboard_ugc: *const c_void,
    pub upload_leaderboard_score: *const c_void,
    pub get_number_of_current_players: *const c_void,
    pub get_num_achieved_achievements:
        unsafe extern "C" fn(*mut IClientUserStatsMap, *const CGameID) -> u32,
    pub get_last_achievement_unlocked: *const c_void,
    pub get_most_recent_achievement_unlocked: *const c_void,
    pub request_global_achievement_percentages:
        unsafe extern "C" fn(*mut IClientUserStatsMap, *const CGameID) -> SteamAPICall_t,
    pub get_most_achieved_achievement_info: *const c_void,
    pub get_next_most_achieved_achievement_info: *const c_void,
    pub get_achievement_achieved_percent: unsafe extern "C" fn(
        *mut IClientUserStatsMap,
        *const CGameID,
        *const c_char,
        *mut f32,
    ) -> bool,
    pub request_global_stats: *const c_void,
    pub get_global_stat_i64: *const c_void,
    pub get_global_stat_f64: *const c_void,
    pub get_global_stat_history_i64: *const c_void,
    pub get_global_stat_history_f64: *const c_void,
    pub get_achievement_progress_limits_i32: *const c_void,
    pub get_achievement_progress_limits_f32: *const c_void,
    pub bachievement_icon_loaded: *const c_void,
}
