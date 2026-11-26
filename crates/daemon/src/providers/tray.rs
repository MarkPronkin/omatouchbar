//! Reads the shell's StatusNotifierWatcher and dispatches taps to the advertising app.

use std::{collections::HashMap, sync::mpsc, thread, time::Duration};

use omatouchbar_core::{
    scene::TrayMenu,
    state::{IconImage, TrayItem, TrayMenuEntry},
};
use zbus::{
    MatchRule,
    blocking::{Connection, connection::Builder},
    message::Type,
    zvariant::{OwnedValue, Value},
};

use crate::bus::{Bus, Event};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const MENU: &str = "com.canonical.dbusmenu";

type Properties = HashMap<String, OwnedValue>;
type MenuNode = (i32, Properties, Vec<OwnedValue>);

fn connection() -> zbus::Result<Connection> {
    Builder::session()?
        .method_timeout(Duration::from_secs(2))
        .build()
}

fn endpoint(address: &str) -> (&str, &str) {
    match address.find('/') {
        Some(i) => address.split_at(i),
        None => (address, "/StatusNotifierItem"),
    }
}

fn string<'a>(props: &'a Properties, key: &str) -> Option<&'a str> {
    <&str>::try_from(props.get(key)?)
        .ok()
        .filter(|s| !s.is_empty())
}

fn flag(props: &Properties, key: &str, default: bool) -> bool {
    props
        .get(key)
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(default)
}

fn read_item(c: &Connection, address: String) -> Option<TrayItem> {
    let (service, path) = endpoint(&address);
    let props: Properties = c
        .call_method(Some(service), path, Some(PROPERTIES), "GetAll", &(ITEM,))
        .ok()?
        .body()
        .deserialize()
        .ok()?;
    parse_item(address, props)
}

fn parse_item(address: String, mut props: Properties) -> Option<TrayItem> {
    let (service, _) = endpoint(&address);
    if props.is_empty() || string(&props, "Status") == Some("Passive") {
        return None;
    }
    let id = string(&props, "Id").unwrap_or(service).to_string();
    let title = string(&props, "Title").unwrap_or(&id).to_string();
    let menu_path = props
        .get("Menu")
        .and_then(|p| <&zbus::zvariant::ObjectPath>::try_from(p).ok())
        .map(|p| p.as_str())
        .filter(|p| !matches!(*p, "/" | "/NO_DBUSMENU"))
        .map(str::to_string);
    let attention = string(&props, "Status") == Some("NeedsAttention");
    let icon = if attention {
        pixmap(props.remove("AttentionIconPixmap"))
    } else {
        None
    }
    .or_else(|| pixmap(props.remove("IconPixmap")));
    Some(TrayItem {
        address,
        id,
        title,
        menu_path,
        icon,
        menu_only: flag(&props, "ItemIsMenu", false),
        attention,
    })
}

fn pixmap(value: Option<OwnedValue>) -> Option<IconImage> {
    let mut choices: Vec<(i32, i32, Vec<u8>)> = value?.try_into().ok()?;
    choices.sort_by_key(|(w, h, _)| w.max(h).abs_diff(32));
    choices
        .into_iter()
        .find_map(|(w, h, data)| IconImage::from_argb(w, h, data))
}

fn items(c: &Connection) -> zbus::Result<Vec<TrayItem>> {
    let reply = c.call_method(
        Some(WATCHER),
        "/StatusNotifierWatcher",
        Some(PROPERTIES),
        "Get",
        &(WATCHER, "RegisteredStatusNotifierItems"),
    )?;
    let addresses: Vec<String> = reply.body().deserialize::<OwnedValue>()?.try_into()?;
    Ok(addresses
        .into_iter()
        .filter_map(|address| read_item(c, address))
        .collect())
}

pub fn spawn(bus: Bus) -> Option<mpsc::Sender<()>> {
    let c = connection().ok()?;
    let rules = [WATCHER, ITEM]
        .into_iter()
        .map(|iface| {
            MatchRule::builder()
                .msg_type(Type::Signal)
                .interface(iface)
                .unwrap()
                .build()
        })
        .collect();
    Some(super::dbus::watch_signals(
        "tray",
        c,
        rules,
        None,
        move |c| {
            let items = items(c).unwrap_or_default();
            bus.patch(move |s| s.tray = items);
        },
    ))
}

fn load_menu(menu: &TrayMenu) -> zbus::Result<Vec<TrayMenuEntry>> {
    let c = connection()?;
    let (service, _) = endpoint(&menu.address);
    let parent = menu.parents.last().copied().unwrap_or(0);
    let _ = c.call_method(
        Some(service),
        menu.path.as_str(),
        Some(MENU),
        "AboutToShow",
        &(parent,),
    );
    let fields = [
        "label",
        "visible",
        "enabled",
        "children-display",
        "toggle-state",
        "type",
    ];
    let reply = c.call_method(
        Some(service),
        menu.path.as_str(),
        Some(MENU),
        "GetLayout",
        &(parent, 1i32, fields.as_slice()),
    )?;
    let (_, (_, _, children)): (u32, MenuNode) = reply.body().deserialize()?;
    Ok(children
        .into_iter()
        .filter_map(|child| {
            let (id, props, children) = MenuNode::try_from(child).ok()?;
            if !flag(&props, "visible", true) || string(&props, "type") == Some("separator") {
                return None;
            }
            let label = string(&props, "label")?;
            // DBusMenu escapes a literal underscore by doubling it.
            let mut chars = label.chars().peekable();
            let label = std::iter::from_fn(|| {
                loop {
                    let c = chars.next()?;
                    if c != '_' {
                        return Some(c);
                    }
                    if chars.next_if_eq(&'_').is_some() {
                        return Some('_');
                    }
                }
            })
            .collect();
            Some(TrayMenuEntry {
                id,
                label,
                enabled: flag(&props, "enabled", true),
                submenu: !children.is_empty()
                    || string(&props, "children-display") == Some("submenu"),
                checked: props
                    .get("toggle-state")
                    .and_then(|v| i32::try_from(v).ok())
                    == Some(1),
            })
        })
        .collect())
}

pub fn request_menu(bus: Bus, menu: TrayMenu) {
    thread::spawn(move || {
        let entries = load_menu(&menu).unwrap_or_else(|e| {
            log::debug!("tray menu: {e}");
            Vec::new()
        });
        bus.send(Event::TrayMenu { menu, entries });
    });
}

pub fn activate(address: String, context_menu: bool) {
    thread::spawn(move || {
        let (service, path) = endpoint(&address);
        let method = if context_menu {
            "ContextMenu"
        } else {
            "Activate"
        };
        if let Err(e) = connection()
            .and_then(|c| c.call_method(Some(service), path, Some(ITEM), method, &(0i32, 0i32)))
        {
            log::warn!("tray {method}: {e}");
        }
    });
}

pub fn click(menu: TrayMenu, id: i32) {
    thread::spawn(move || {
        let (service, _) = endpoint(&menu.address);
        let time = chrono::Utc::now().timestamp_millis() as u32;
        if let Err(e) = connection().and_then(|c| {
            c.call_method(
                Some(service),
                menu.path.as_str(),
                Some(MENU),
                "Event",
                &(id, "clicked", Value::I32(0), time),
            )
        }) {
            log::warn!("tray menu action: {e}");
        }
    });
}
