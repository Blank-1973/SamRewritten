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

use crate::gui_frontend::MainApplication;
use crate::gui_frontend::achievement_automatic_view::create_achievements_automatic_view;
use crate::gui_frontend::achievement_manual_view::create_achievements_manual_view;
use crate::gui_frontend::gobjects::achievement::GAchievementObject;
use crate::gui_frontend::gsettings::get_settings;
use crate::gui_frontend::unlock_scheduler::AchievementModelUpdates;
use gtk::gio::{ListStore, SimpleAction};
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    CustomFilter, CustomSorter, FilterChange, FilterListModel, Label, NoSelection, SortListModel,
    SorterChange, Stack, StackTransitionType, StringFilter, StringFilterMatchMode,
};
use std::cell::Cell;
use std::cmp::Ordering;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Copy, Default)]
enum AchievementOrder {
    SteamDefault,
    Alphabetical,
    #[default]
    GlobalPercentage,
    UnlockDate,
}

impl AchievementOrder {
    fn from_action_target(value: &str) -> Option<Self> {
        match value {
            "steam-default" => Some(Self::SteamDefault),
            "alphabetical" => Some(Self::Alphabetical),
            "global-percentage" => Some(Self::GlobalPercentage),
            "unlock-date" => Some(Self::UnlockDate),
            _ => None,
        }
    }

    fn as_action_target(self) -> &'static str {
        match self {
            Self::SteamDefault => "steam-default",
            Self::Alphabetical => "alphabetical",
            Self::GlobalPercentage => "global-percentage",
            Self::UnlockDate => "unlock-date",
        }
    }
}

pub fn create_achievements_view(
    app_id: Rc<Cell<Option<u32>>>,
    app_unlocked_achievements_count: Rc<Cell<usize>>,
    application: &MainApplication,
    app_achievement_count_value: &Label,
) -> (Stack, ListStore, StringFilter, Arc<AtomicBool>) {
    let settings = get_settings();
    let app_achievements_model = ListStore::new::<GAchievementObject>();
    let app_timed_achievements_model = ListStore::new::<GAchievementObject>();

    let app_achievement_string_filter = StringFilter::builder()
        .expression(GAchievementObject::this_expression("search-text"))
        .match_mode(StringFilterMatchMode::Substring)
        .ignore_case(true)
        .build();
    let app_achievement_filter_model = FilterListModel::builder()
        .model(&app_achievements_model)
        .filter(&app_achievement_string_filter)
        .build();
    // None shows all achievements; Some(false) and Some(true) show only
    // locked and unlocked achievements respectively.
    let achievement_status = Rc::new(Cell::new(None));
    let achievement_status_filter = CustomFilter::new({
        let achievement_status = Rc::clone(&achievement_status);
        move |object| {
            let achievement = object.downcast_ref::<GAchievementObject>().unwrap();
            achievement_status
                .get()
                .is_none_or(|unlocked| achievement.is_achieved() == unlocked)
        }
    });
    let app_achievement_status_filter_model = FilterListModel::builder()
        .model(&app_achievement_filter_model)
        .filter(&achievement_status_filter)
        .build();
    let app_achievement_timed_filter_model = FilterListModel::builder()
        .model(&app_timed_achievements_model)
        .filter(&app_achievement_string_filter)
        .build();

    let initial_order = AchievementOrder::from_action_target(&settings.string("achievement-order"))
        .unwrap_or_default();
    let achievement_order = Rc::new(Cell::new(initial_order));
    let achievement_sorter = CustomSorter::new({
        let achievement_order = Rc::clone(&achievement_order);
        move |obj1, obj2| {
            let achievement1 = obj1.downcast_ref::<GAchievementObject>().unwrap();
            let achievement2 = obj2.downcast_ref::<GAchievementObject>().unwrap();
            match achievement_order.get() {
                AchievementOrder::SteamDefault => Ordering::Equal.into(),
                AchievementOrder::Alphabetical => achievement1
                    .name()
                    .to_lowercase()
                    .cmp(&achievement2.name().to_lowercase())
                    .into(),
                AchievementOrder::UnlockDate => achievement2
                    .unlock_time_seconds()
                    .cmp(&achievement1.unlock_time_seconds())
                    .into(),
                AchievementOrder::GlobalPercentage => achievement2
                    .global_achieved_percent()
                    .partial_cmp(&achievement1.global_achieved_percent())
                    .unwrap_or(Ordering::Equal)
                    .into(),
            }
        }
    });
    let app_achievement_sort_model = SortListModel::builder()
        .model(&app_achievement_status_filter_model)
        .sorter(&achievement_sorter)
        .build();
    if matches!(initial_order, AchievementOrder::SteamDefault) {
        app_achievement_sort_model.set_sorter(None::<&CustomSorter>);
    }
    let achievement_model_updates =
        AchievementModelUpdates::new(&achievement_status_filter, &achievement_sorter);

    let order_action = SimpleAction::new_stateful(
        "achievement-order",
        Some(&String::static_variant_type()),
        &initial_order.as_action_target().to_variant(),
    );
    order_action.connect_activate(glib::clone!(
        #[strong]
        achievement_order,
        #[strong]
        achievement_sorter,
        #[strong]
        settings,
        #[weak(rename_to = sort_model)]
        app_achievement_sort_model,
        move |action, target| {
            let Some(value) = target.and_then(|target| target.str()) else {
                return;
            };
            let Some(order) = AchievementOrder::from_action_target(value) else {
                return;
            };
            action.set_state(&value.to_variant());
            if let Err(e) = settings.set_string("achievement-order", value) {
                eprintln!("[CLIENT] Error saving achievement order: {e:?}");
            }
            achievement_order.set(order);
            if matches!(order, AchievementOrder::SteamDefault) {
                sort_model.set_sorter(None::<&CustomSorter>);
            } else {
                sort_model.set_sorter(Some(&achievement_sorter));
                achievement_sorter.changed(SorterChange::Different);
            }
        }
    ));
    application.add_action(&order_action);

    let state_action = SimpleAction::new_stateful(
        "achievement-state",
        Some(&String::static_variant_type()),
        &"all".to_variant(),
    );
    state_action.connect_activate(glib::clone!(
        #[strong]
        achievement_status,
        #[weak]
        achievement_status_filter,
        move |action, target| {
            let Some(value) = target.and_then(|target| target.str()) else {
                return;
            };
            let state = match value {
                "all" => None,
                "locked" => Some(false),
                "unlocked" => Some(true),
                _ => return,
            };
            action.set_state(&value.to_variant());
            achievement_status.set(state);
            achievement_status_filter.changed(FilterChange::Different);
        }
    ));
    application.add_action(&state_action);

    let app_achievement_selection_model = NoSelection::new(Option::<ListStore>::None);
    app_achievement_selection_model.set_model(Some(&app_achievement_sort_model));
    let app_timed_achievement_selection_model = NoSelection::new(Option::<ListStore>::None);
    app_timed_achievement_selection_model.set_model(Some(&app_achievement_timed_filter_model));

    let achievement_views_stack = Stack::builder()
        .transition_type(StackTransitionType::SlideLeftRight)
        .build();
    let (achievements_manual_frame, cancel_timed_unlock) = create_achievements_manual_view(
        &app_id,
        &app_unlocked_achievements_count,
        &app_achievement_selection_model,
        &achievement_model_updates,
        &app_achievements_model,
        &app_timed_achievements_model,
        &achievement_views_stack,
        app_achievement_count_value,
        application,
    );
    let (achievements_automatic_frame, _achievements_automatic_stop) =
        create_achievements_automatic_view(&app_timed_achievement_selection_model, application);

    achievement_views_stack.add_named(&achievements_manual_frame, Some("manual"));
    achievement_views_stack.add_named(&achievements_automatic_frame, Some("automatic"));

    (
        achievement_views_stack,
        app_achievements_model,
        app_achievement_string_filter,
        cancel_timed_unlock,
    )
}

#[cfg(test)]
mod tests {
    use super::AchievementOrder;

    #[test]
    fn achievement_order_rejects_unknown_action_targets() {
        assert!(AchievementOrder::from_action_target("unknown").is_none());
    }
}
