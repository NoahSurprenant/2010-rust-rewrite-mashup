use bevy::prelude::*;
use frame::ClientSet;

use crate::classes::store::{
    ClassStoreFile, load_class_store, save_class_store, sync_host_class_loadouts,
};

#[derive(Resource, Clone, Debug, Default)]
pub struct MenuMapList(pub Vec<asset_transport::MapPack>);

impl MenuMapList {
    pub fn maps(&self) -> impl Iterator<Item = &String> {
        self.0.iter().flat_map(|pack| &pack.maps)
    }

    pub fn contains(&self, map: &str) -> bool {
        self.maps().any(|installed| installed == map)
    }

    pub fn pack_of(&self, map: &str) -> Option<usize> {
        self.0
            .iter()
            .position(|pack| pack.maps.iter().any(|installed| installed == map))
    }
}

pub(crate) struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuMapList>()
            .init_resource::<crate::ClassLoadoutCatalog>()
            .init_resource::<frame::GameSettings>()
            .init_resource::<crate::BindingView>()
            .init_resource::<crate::SessionClassStore>()
            .init_resource::<ClassStoreFile>()
            .init_resource::<frame::HostClassLoadouts>()
            .init_resource::<asset_game::MenuCatalog>()
            .init_resource::<asset_game::LocalizeCatalog>()
            .add_systems(
                Update,
                (
                    crate::options::apply_window_settings,
                    load_class_store,
                    sync_host_class_loadouts,
                    save_class_store,
                )
                    .chain()
                    .in_set(ClientSet::Ui),
            );
    }
}

pub fn install_frontend_menus(catalog: &mut asset_game::MenuCatalog) -> Result<(), String> {
    catalog.load_definitions(include_str!("../menus/frontend.json"))?;
    catalog.load_definitions(include_str!("../menus/connection_error.json"))?;
    catalog.load_definitions(include_str!("../menus/classes.json"))?;
    catalog.load_definitions(include_str!("../menus/settings.json"))?;
    catalog.load_definitions(include_str!("../menus/controller.json"))?;
    let has_controller_page = catalog.get("options_controller").is_some();
    for (name, menu) in &mut catalog.menus {
        if matches!(name.as_str(), "popup_endgame" | "popup_endgame_ranked") {
            for item in &mut menu.items {
                if item.name == "button_yes" {
                    item.handlers.action = vec![asset_game::MenuEvent::Script(
                        "play mouse_click; close self; exec \"disconnect\";".into(),
                    )];
                }
            }
        }
        if let Some(settings_link) = menu
            .items
            .iter()
            .find(|item| {
                item.item_type == 1
                    && matches!(item.text_key.as_str(), "@MENU_CHAT" | "@MENU_VOICE")
            })
            .cloned()
            && menu
                .items
                .iter()
                .any(|item| item.text_key == "@MENU_RESET_SYSTEM_DEFAULTS")
        {
            let mut multiplayer = settings_link;
            multiplayer.name = "multiplayer_settings".into();
            multiplayer.text_key = "@MENU_MULTIPLAYER_OPTIONS".into();
            multiplayer.rect.y = 88.0;
            multiplayer.vis_exp = "1".into();
            multiplayer.disabled_exp = "0".into();
            multiplayer.handlers.action = vec![asset_game::MenuEvent::Script(
                "play mouse_click; close self; open options_multi;".into(),
            )];
            let mut controller = multiplayer.clone();
            menu.items.push(multiplayer);
            if has_controller_page {
                controller.name = "controller_settings".into();
                controller.text_key = "Controller".into();
                controller.rect.y = 108.0;
                controller.handlers.action = vec![asset_game::MenuEvent::Script(
                    "play mouse_click; close self; open options_controller;".into(),
                )];
                menu.items.push(controller);
            }
        }

        let removed_rows: Vec<_> = menu
            .items
            .iter()
            .filter(|item| {
                item.item_type == 1
                    && matches!(
                        item.text_key.as_str(),
                        "@MENU_VOICE" | "@MENU_CHAT" | "@MENU_RESET_SYSTEM_DEFAULTS"
                    )
            })
            .map(|item| (item.rect.x, item.rect.y))
            .collect();
        menu.items.retain(|item| {
            !removed_rows.iter().any(|&(x, y)| {
                item.rect.x == x
                    && item.rect.y == y
                    && item.name != "multiplayer_settings"
                    && item.name != "controller_settings"
            })
        });
        if name == "options_multi" {
            add_player_key_rows(menu);
        }
        if name == "pc_options_controls" {
            for item in &mut menu.items {
                if item.rect.x >= 232.0 && item.rect.y > 88.0 {
                    item.rect.y -= 20.0;
                }
            }
        }

        let Some(mode) = name.strip_prefix("settings_quick_") else {
            continue;
        };
        if sim::HostGameModeSelection::from_token(mode).is_none() {
            continue;
        }
        for item in &mut menu.items {
            if item.dvar == "camera_thirdperson" {
                if item.item_type == 12 {
                    item.choices.clear();
                    item.dvar.clear();
                    item.text_key = "Недоступно".into();
                }
                item.disabled_exp = "1".into();
                item.item_type = 0;
                item.static_flags |= 0x100000;
                item.handlers.action.clear();
                item.fore_color[3] = 0.4;
            } else if item.item_type == 1 && !item.dvar.is_empty() {
                item.text_scale = 0.30;
            }
        }
    }
    Ok(())
}

/// A menu expression that shows the dvar `name`'s value.
fn dvar_text(name: &str) -> String {
    let hex: String = name.bytes().map(|b| format!("{b:02x}")).collect();
    format!("op 38 s:{hex} op 1")
}

/// Under the player name on the multiplayer options page: this game's player
/// key, short (`ui_player_key`), as a button that copies the whole key
/// (`whoami copy`), and a line saying what the button did
/// (`ui_player_key_note`). Styled as the name's label and field.
fn add_player_key_rows(menu: &mut asset_game::MenuDef) {
    let label = menu
        .items
        .iter()
        .find(|item| item.text_key == "@MENU_PLAYER_NAME")
        .cloned();
    let field = menu
        .items
        .iter()
        .find(|item| item.dvar == "ui_player_name")
        .cloned();
    let (Some(label), Some(field)) = (label, field) else {
        return;
    };
    let mut key_label = label.clone();
    key_label.name = "player_key_label".into();
    key_label.text_key = "Player key".into();
    key_label.rect.y += 20.0;

    let mut key = field;
    key.name = "player_key".into();
    key.item_type = 1;
    key.style = 1;
    key.edit_field = None;
    key.dvar.clear();
    key.text_key.clear();
    key.text_exp = dvar_text("ui_player_key");
    key.rect.y += 20.0;
    key.background = "menu_button_selection_bar".into();
    key.back_color = [0.0, 0.0, 0.0, 0.0];
    key.handlers.action = vec![asset_game::MenuEvent::Script(
        "play mouse_click; exec \"whoami copy\";".into(),
    )];
    key.handlers.accept.clear();
    key.handlers.focus = vec![asset_game::MenuEvent::Script(
        "play mouse_over; setItemColor self backcolor 0 0 0 1;".into(),
    )];
    key.handlers.leave_focus = vec![asset_game::MenuEvent::Script(
        "setItemColor self backcolor 0 0 0 0;".into(),
    )];

    let mut note = label;
    note.name = "player_key_note".into();
    note.text_key.clear();
    note.text_exp = dvar_text("ui_player_key_note");
    note.rect.y += 40.0;
    note.text_scale *= 0.85;
    note.fore_color[3] *= 0.7;

    menu.items.extend([key_label, key, note]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options_multi() -> asset_game::MenuDef {
        let mut label = asset_game::MenuItem {
            text_key: "@MENU_PLAYER_NAME".into(),
            text_scale: 0.32,
            fore_color: [1.0; 4],
            ..Default::default()
        };
        label.rect.x = 232.0;
        label.rect.y = 28.0;
        let mut field = asset_game::MenuItem {
            dvar: "ui_player_name".into(),
            item_type: 4,
            edit_field: Some(Default::default()),
            ..Default::default()
        };
        field.rect.x = 420.0;
        field.rect.y = 28.0;
        let mut menu = asset_game::MenuDef::default();
        menu.name = "options_multi".into();
        menu.items = vec![label, field];
        menu
    }

    #[test]
    fn the_player_key_shows_under_the_name_and_its_button_copies_it() {
        let mut menu = options_multi();
        add_player_key_rows(&mut menu);
        let item = |name: &str| {
            menu.items
                .iter()
                .find(|item| item.name == name)
                .unwrap_or_else(|| panic!("no {name}"))
        };
        let key = item("player_key");
        assert_eq!(key.item_type, 1);
        assert!(key.edit_field.is_none() && key.dvar.is_empty());
        assert_eq!((key.rect.x, key.rect.y), (420.0, 48.0));
        assert_eq!(key.text_exp, "op 38 s:75695f706c617965725f6b6579 op 1");
        let action = match key.handlers.action.as_slice() {
            [asset_game::MenuEvent::Script(script)] => script.clone(),
            other => panic!("{other:?}"),
        };
        assert!(action.contains("exec \"whoami copy\""), "{action}");
        assert_eq!(item("player_key_label").text_key, "Player key");
        assert_eq!(item("player_key_label").rect.y, 48.0);
        assert_eq!(item("player_key_note").rect.y, 68.0);
        assert!(item("player_key_note").text_exp.contains("5f6e6f7465"));
        // The name row is untouched.
        assert_eq!(menu.items[1].dvar, "ui_player_name");
        assert!(menu.items[1].edit_field.is_some());
    }

    #[test]
    fn a_page_without_the_name_row_is_left_alone() {
        let mut menu = options_multi();
        menu.items.retain(|item| item.dvar.is_empty());
        add_player_key_rows(&mut menu);
        assert_eq!(menu.items.len(), 1);
    }
}
