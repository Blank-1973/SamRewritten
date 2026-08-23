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

use crate::steam_client::client_engine_wrapper::ClientEngineInner;
use crate::steam_client::client_user_stats_map_vtable::{
    IClientUserStatsMap, IClientUserStatsMapVTable,
};
use crate::steam_client::create_client::{api_call_result_fetcher, callback_pump};
use crate::steam_client::steamworks_types::{
    AppId_t, CGameID, CSteamID, HSteamPipe, SteamAPICall_t, SteamAPICallCompleted_t,
    SteamCallbackMessage, UserStatsReceived_t,
};
use crate::steam_client::wrapper_types::{SteamCallbackId, SteamClientError};
use std::ffi::CString;
use std::mem::offset_of;
use std::os::raw::{c_int, c_void};
use std::rc::Rc;

pub struct ClientUserStatsMap {
    inner: Rc<ClientUserStatsMapInner>,
}

struct ClientUserStatsMapInner {
    ptr: *mut IClientUserStatsMap,
    engine: Rc<ClientEngineInner>,
    pipe: HSteamPipe,
}

impl ClientUserStatsMapInner {
    fn vtable(&self) -> Result<&IClientUserStatsMapVTable, SteamClientError> {
        unsafe { (*self.ptr).vtable.as_ref() }.ok_or(SteamClientError::NullVtable)
    }
}

impl ClientUserStatsMap {
    pub unsafe fn from_raw(
        ptr: *mut IClientUserStatsMap,
        engine: Rc<ClientEngineInner>,
        pipe: HSteamPipe,
    ) -> Self {
        Self {
            inner: Rc::new(ClientUserStatsMapInner { ptr, engine, pipe }),
        }
    }

    /// Result kept raw: an `EResult` Valve added since is not a valid enum.
    pub fn drain_user_stats_callbacks(&self) -> Vec<(AppId_t, i32)> {
        let Some((get, free)) = callback_pump() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        loop {
            let mut msg = SteamCallbackMessage {
                user: 0,
                id: 0,
                param_ptr: std::ptr::null_mut(),
                param_size: 0,
            };
            let mut call: c_int = 0;
            unsafe {
                if !get(self.inner.pipe, &mut msg, &mut call) {
                    break;
                }
                if msg.id == SteamCallbackId::UserStatsReceived as c_int
                    && !msg.param_ptr.is_null()
                    && msg.param_size >= size_of::<UserStatsReceived_t>() as c_int
                {
                    let base = msg.param_ptr as *const u8;
                    let game = base.add(offset_of!(UserStatsReceived_t, m_nGameID));
                    let result = base.add(offset_of!(UserStatsReceived_t, m_eResult));
                    out.push((
                        std::ptr::read_unaligned(game as *const u64) as AppId_t,
                        std::ptr::read_unaligned(result as *const i32),
                    ));
                }
                free(self.inner.pipe);
            }
        }
        out
    }

    /// The pipe only announces the handle; the payload is fetched separately.
    /// Draining is destructive, so other queued callbacks are dropped meanwhile.
    pub fn wait_for_api_call<T>(
        &self,
        handle: SteamAPICall_t,
        expected: SteamCallbackId,
        frames: u32,
    ) -> Option<T> {
        let (get, free) = callback_pump()?;
        let fetch = api_call_result_fetcher()?;
        let expected = expected as c_int;

        for _ in 0..frames {
            self.run_engine_frame();
            loop {
                let mut msg = SteamCallbackMessage {
                    user: 0,
                    id: 0,
                    param_ptr: std::ptr::null_mut(),
                    param_size: 0,
                };
                let mut call: c_int = 0;
                unsafe {
                    if !get(self.inner.pipe, &mut msg, &mut call) {
                        break;
                    }
                    let completed = msg.id == SteamCallbackId::ApiCallCompleted as c_int
                        && !msg.param_ptr.is_null()
                        && msg.param_size >= size_of::<SteamAPICallCompleted_t>() as c_int
                        && std::ptr::read_unaligned(
                            (msg.param_ptr as *const u8)
                                .add(offset_of!(SteamAPICallCompleted_t, m_hAsyncCall))
                                as *const SteamAPICall_t,
                        ) == handle;

                    if !completed {
                        free(self.inner.pipe);
                        continue;
                    }

                    let mut payload = std::mem::MaybeUninit::<T>::zeroed();
                    let mut failed = false;
                    let ok = fetch(
                        self.inner.pipe,
                        handle,
                        payload.as_mut_ptr() as *mut c_void,
                        size_of::<T>() as c_int,
                        expected,
                        &mut failed,
                    );
                    free(self.inner.pipe);
                    return if ok && !failed {
                        Some(payload.assume_init())
                    } else {
                        None
                    };
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(17));
        }
        None
    }

    pub fn run_engine_frame(&self) {
        self.inner.engine.run_frame();
    }

    pub fn request_current_stats(&self, app_id: AppId_t) -> Result<bool, SteamClientError> {
        let gid = CGameID::from_app_id(app_id);
        unsafe {
            let vt = self.inner.vtable()?;
            Ok((vt.request_current_stats)(self.inner.ptr, &gid))
        }
    }

    pub fn get_num_achievements(&self, app_id: AppId_t) -> Result<u32, SteamClientError> {
        let gid = CGameID::from_app_id(app_id);
        unsafe {
            let vt = self.inner.vtable()?;
            Ok((vt.get_num_achievements)(self.inner.ptr, &gid))
        }
    }

    pub fn get_num_achieved_achievements(&self, app_id: AppId_t) -> Result<u32, SteamClientError> {
        let gid = CGameID::from_app_id(app_id);
        unsafe {
            let vt = self.inner.vtable()?;
            Ok((vt.get_num_achieved_achievements)(self.inner.ptr, &gid))
        }
    }

    pub fn request_user_stats(
        &self,
        steam_id: CSteamID,
        app_id: AppId_t,
    ) -> Result<SteamAPICall_t, SteamClientError> {
        let gid = CGameID::from_app_id(app_id);
        unsafe {
            let vt = self.inner.vtable()?;
            Ok((vt.request_user_stats)(self.inner.ptr, steam_id, &gid))
        }
    }

    pub fn get_user_achievement_and_unlock_time(
        &self,
        steam_id: CSteamID,
        app_id: AppId_t,
        achievement_name: &str,
    ) -> Result<(bool, u32), SteamClientError> {
        let gid = CGameID::from_app_id(app_id);
        let name = CString::new(achievement_name).map_err(|_| SteamClientError::UnknownError)?;
        unsafe {
            let vt = self.inner.vtable()?;
            let mut achieved = false;
            let mut unlock_time = 0u32;
            let ok = (vt.get_user_achievement)(
                self.inner.ptr,
                steam_id,
                &gid,
                name.as_ptr(),
                &mut achieved,
                &mut unlock_time,
            );
            if ok {
                Ok((achieved, unlock_time))
            } else {
                Err(SteamClientError::UnknownError)
            }
        }
    }

    pub fn get_achievement_and_unlock_time(
        &self,
        app_id: AppId_t,
        achievement_name: &str,
    ) -> Result<(bool, u32), SteamClientError> {
        let gid = CGameID::from_app_id(app_id);
        let name = CString::new(achievement_name).map_err(|_| SteamClientError::UnknownError)?;
        unsafe {
            let vt = self.inner.vtable()?;
            let mut achieved = false;
            let mut unlock_time = 0u32;
            let ok = (vt.get_achievement)(
                self.inner.ptr,
                &gid,
                name.as_ptr(),
                &mut achieved,
                &mut unlock_time,
            );
            if ok {
                Ok((achieved, unlock_time))
            } else {
                Err(SteamClientError::UnknownError)
            }
        }
    }

    pub fn set_achievement(
        &self,
        app_id: AppId_t,
        achievement_name: &str,
    ) -> Result<(), SteamClientError> {
        let gid = CGameID::from_app_id(app_id);
        let name = CString::new(achievement_name).map_err(|_| SteamClientError::UnknownError)?;
        unsafe {
            let vt = self.inner.vtable()?;
            if (vt.set_achievement)(self.inner.ptr, &gid, name.as_ptr()) {
                Ok(())
            } else {
                Err(SteamClientError::UnknownError)
            }
        }
    }

    pub fn clear_achievement(
        &self,
        app_id: AppId_t,
        achievement_name: &str,
    ) -> Result<(), SteamClientError> {
        let gid = CGameID::from_app_id(app_id);
        let name = CString::new(achievement_name).map_err(|_| SteamClientError::UnknownError)?;
        unsafe {
            let vt = self.inner.vtable()?;
            if (vt.clear_achievement)(self.inner.ptr, &gid, name.as_ptr()) {
                Ok(())
            } else {
                Err(SteamClientError::UnknownError)
            }
        }
    }

    pub fn get_stat_i32(&self, app_id: AppId_t, stat_name: &str) -> Result<i32, SteamClientError> {
        let gid = CGameID::from_app_id(app_id);
        let name = CString::new(stat_name).map_err(|_| SteamClientError::UnknownError)?;
        unsafe {
            let vt = self.inner.vtable()?;
            let mut value = 0i32;
            if (vt.get_stat_i32)(self.inner.ptr, &gid, name.as_ptr(), &mut value) {
                Ok(value)
            } else {
                Err(SteamClientError::UnknownError)
            }
        }
    }

    pub fn get_stat_f32(&self, app_id: AppId_t, stat_name: &str) -> Result<f32, SteamClientError> {
        let gid = CGameID::from_app_id(app_id);
        let name = CString::new(stat_name).map_err(|_| SteamClientError::UnknownError)?;
        unsafe {
            let vt = self.inner.vtable()?;
            let mut value = 0f32;
            if (vt.get_stat_f32)(self.inner.ptr, &gid, name.as_ptr(), &mut value) {
                Ok(value)
            } else {
                Err(SteamClientError::UnknownError)
            }
        }
    }

    /// `false` is the client refusing the write (e.g. an increment-only stat), not
    /// a transport error. `store_stats` still returns true, so this is the only signal.
    pub fn set_stat_i32(
        &self,
        app_id: AppId_t,
        stat_name: &str,
        value: i32,
    ) -> Result<i32, SteamClientError> {
        let gid = CGameID::from_app_id(app_id);
        let name = CString::new(stat_name).map_err(|_| SteamClientError::UnknownError)?;
        unsafe {
            let vt = self.inner.vtable()?;
            if (vt.set_stat_i32)(self.inner.ptr, &gid, name.as_ptr(), value) {
                Ok(value)
            } else {
                Err(SteamClientError::UnknownError)
            }
        }
    }

    pub fn set_stat_f32(
        &self,
        app_id: AppId_t,
        stat_name: &str,
        value: f32,
    ) -> Result<f32, SteamClientError> {
        let gid = CGameID::from_app_id(app_id);
        let name = CString::new(stat_name).map_err(|_| SteamClientError::UnknownError)?;
        unsafe {
            let vt = self.inner.vtable()?;
            if (vt.set_stat_f32)(self.inner.ptr, &gid, name.as_ptr(), value) {
                Ok(value)
            } else {
                Err(SteamClientError::UnknownError)
            }
        }
    }

    pub fn store_stats(&self, app_id: AppId_t) -> Result<bool, SteamClientError> {
        let gid = CGameID::from_app_id(app_id);
        unsafe {
            let vt = self.inner.vtable()?;
            Ok((vt.store_stats)(self.inner.ptr, &gid))
        }
    }

    pub fn reset_all_stats(
        &self,
        app_id: AppId_t,
        achievements_too: bool,
    ) -> Result<bool, SteamClientError> {
        let gid = CGameID::from_app_id(app_id);
        unsafe {
            let vt = self.inner.vtable()?;
            Ok((vt.reset_all_stats)(self.inner.ptr, &gid, achievements_too))
        }
    }

    pub fn request_global_achievement_percentages(
        &self,
        app_id: AppId_t,
    ) -> Result<SteamAPICall_t, SteamClientError> {
        let gid = CGameID::from_app_id(app_id);
        unsafe {
            let vt = self.inner.vtable()?;
            Ok((vt.request_global_achievement_percentages)(
                self.inner.ptr,
                &gid,
            ))
        }
    }

    pub fn get_achievement_achieved_percent(
        &self,
        app_id: AppId_t,
        achievement_name: &str,
    ) -> Result<f32, SteamClientError> {
        let gid = CGameID::from_app_id(app_id);
        let name = CString::new(achievement_name).map_err(|_| SteamClientError::UnknownError)?;
        unsafe {
            let vt = self.inner.vtable()?;
            let mut percent = 0f32;
            if (vt.get_achievement_achieved_percent)(
                self.inner.ptr,
                &gid,
                name.as_ptr(),
                &mut percent,
            ) {
                Ok(percent)
            } else {
                Err(SteamClientError::UnknownError)
            }
        }
    }
}
