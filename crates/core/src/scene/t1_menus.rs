use super::*;
use crate::{Canvas, T1_HEIGHT, T1_WIDTH};

fn scene(menu: MenuTree) -> Scene {
    let mut config = Config {
        mode: Mode::Bar,
        ..Config::default()
    };
    config.motion.enabled = false;
    let mut scene = Scene::new(
        T1_WIDTH,
        T1_HEIGHT,
        config,
        Palette::fallback(),
        Fonts::system(None),
        menu,
        None,
    );
    scene.set_state(DesktopState::sample_for_model("MacBookPro14,2"));
    scene.update(scene.now, scene.utc);
    scene
}

/// Scroll just far enough to show the whole control. Coordinates must work
/// both for drawing the row and for tapping it on the physical strip.
fn reveal(scene: &mut Scene, key: Key) -> Rect {
    let node = scene
        .animator
        .get(key)
        .unwrap_or_else(|| panic!("missing {key:?} on {:?}", scene.page));
    let rect = node.rect();
    let interactive = node.node.hit.is_some();
    let (left, right, rect) = if node.node.group == Some(0) {
        let (left, right) = (scene.scroll.x0, scene.scroll.x1);
        scene.set_scroll_offset(rect.right() - right);
        (left, right, rect.offset(-scene.scroll.offset))
    } else {
        (0.0, scene.width, rect)
    };
    assert!(rect.w > 0.0 && rect.h > 0.0 && rect.y >= 0.0 && rect.y + rect.h <= scene.height);
    assert!(
        rect.x >= left - 0.01 && rect.right() <= right + 0.01,
        "{key:?} on {:?} at {rect:?} outside {left}..{right}",
        scene.page
    );
    if interactive {
        assert_eq!(
            scene.hit_test(rect.cx(), rect.cy(), 10.0),
            Some(key),
            "covered control {key:?} on {:?}",
            scene.page
        );
    }
    rect
}

fn tap(scene: &mut Scene, key: Key) {
    let rect = reveal(scene, key);
    scene.input(false, &[(0, rect.cx(), rect.cy())], scene.now);
    scene.input(false, &[], scene.now);
    scene.update(scene.now, scene.utc);
}

fn assert_content_drawn(scene: &Scene, canvas: &mut Canvas) {
    scene.render(canvas);
    let pixels = canvas.pixmap().pixels();
    let start = scene.node_rect(Key::new("nav")).unwrap().right().ceil() as usize;
    let bg = pixels[0];
    assert!(
        pixels
            .chunks_exact(canvas.width() as usize)
            .any(|row| row[start..].iter().any(|&p| p != bg)),
        "blank content on {:?}",
        scene.page
    );
}

#[test]
fn retina_scale_choices_for_both_t1_chassis() {
    assert_eq!((T1_WIDTH, T1_HEIGHT), (2170, 60));
    for model in [
        "MacBookPro13,2",
        "MacBookPro14,2",
        "MacBookPro13,3",
        "MacBookPro14,3",
    ] {
        let state = DesktopState::sample_for_model(model);
        let monitor = state.display_panel.as_ref().unwrap().focused().unwrap();
        let (size, three_x) = if model.ends_with(",2") {
            ((2560, 1600), (300, 320))
        } else {
            ((2880, 1800), (300, 300))
        };
        assert_eq!((monitor.width, monitor.height), size, "{model}");
        assert!(monitor.scales().contains(&three_x), "{model}");
    }
}

#[test]
fn every_panel_renders_and_its_controls_are_reachable() {
    let mut scene = scene(MenuTree::default());
    let mut canvas = Canvas::new(T1_WIDTH, T1_HEIGHT);
    let panels = [
        (Panel::Audio, Key::new("p/volume")),
        (
            Panel::Brightness,
            Key::new("p/display/scale").with("opt").with("320"),
        ),
        (Panel::Power, Key::new("p/profile").with("performance")),
        (Panel::Network, Key::new("p/net/stat/gateway")),
        (Panel::Bluetooth, Key::new("p/bt")),
        (Panel::Clock, Key::new("time/date")),
        (
            Panel::Tray,
            Key::new("p/tray/item").with(&scene.state.tray[0].address),
        ),
        (Panel::Weather, Key::new("p/weather").with("place")),
        (Panel::Agents, Key::new("p/agents/refresh")),
        (Panel::Indicators, Key::new("p/ind/night")),
    ];
    for scale in [1.0, 1.3] {
        let mut config = scene.config().clone();
        config.appearance.text_scale = scale;
        scene.set_config(config);
        for (panel, control) in panels {
            scene.command(Command::OpenPanel(panel));
            scene.update(scene.now, scene.utc);
            assert_eq!(scene.page, Page::Panel(panel));
            assert_content_drawn(&scene, &mut canvas);
            reveal(&mut scene, control);
            let controls: Vec<_> = scene
                .animator
                .items
                .iter()
                .filter(|a| a.node.hit.is_some())
                .map(|a| a.node.key)
                .collect();
            for key in controls {
                reveal(&mut scene, key);
            }
            scene.set_scroll_offset(scene.scroll.max);
            assert_content_drawn(&scene, &mut canvas);
            tap(&mut scene, Key::new("nav"));
            assert_eq!(
                scene.page,
                Page::Bar,
                "Back from {panel:?} at the end of the row"
            );
        }
    }
}

#[test]
fn swipe_a_provider_menu_and_return_to_its_parent() {
    let tree = MenuTree::from_jsonc(
        r#"{
        "setup": {"label":"Setup"},
        "setup.font": {"label":"Font","provider":"fonts"},
        "setup.hidden": {"label":"Hidden","action":"hidden","when":"false"}
    }"#,
    )
    .unwrap();
    let mut scene = scene(tree);
    tap(&mut scene, Key::new("w/menu"));
    tap(&mut scene, Key::new("m").with("setup"));
    scene.apply_guards(
        "setup",
        vec![("setup.hidden".into(), GuardKind::When, false)],
    );
    scene.update(scene.now, scene.utc);
    assert!(
        scene
            .node_rect(Key::new("m").with("setup.hidden"))
            .is_none()
    );
    tap(&mut scene, Key::new("m").with("setup.font"));
    let rows = (0..80)
        .map(|i| (format!("Font family {i}"), i.to_string(), i == 0))
        .collect();
    scene.apply_provider("setup.font", "fonts", rows);
    scene.update(scene.now, scene.utc);
    let y = scene.height / 2.0;
    for x in [1700.0, 1650.0, 450.0] {
        scene.input(false, &[(0, x, y)], scene.now);
    }
    scene.input(false, &[], scene.now);
    assert!(scene.scroll.offset > 1000.0);
    tap(&mut scene, Key::new("nav"));
    assert_eq!(scene.page, Page::Menu("setup".into()));
}

#[test]
fn tray_menu_and_its_submenu_keep_their_back_target() {
    let mut scene = scene(MenuTree::default());
    let mut canvas = Canvas::new(T1_WIDTH, T1_HEIGHT);
    let address = scene.state.tray[0].address.clone();
    scene.command(Command::OpenPanel(Panel::Tray));
    scene.update(scene.now, scene.utc);
    tap(
        &mut scene,
        Key::new("p/tray/item").with(&address).with("menu"),
    );
    let root = scene.tray_menu.clone().unwrap();
    let entry = |id, submenu| TrayMenuEntry {
        id,
        label: format!("Player option {id}"),
        enabled: true,
        checked: false,
        submenu,
    };
    scene.apply_tray_menu(&root, vec![entry(7, true)]);
    scene.update(scene.now, scene.utc);
    let key = Key::new("p/tray/entry").with(&address);
    tap(&mut scene, key.index(7));
    let child = scene.tray_menu.clone().unwrap();
    assert_eq!(child.parents, [0, 7]);
    scene.apply_tray_menu(&child, (10..50).map(|i| entry(i, false)).collect());
    scene.update(scene.now, scene.utc);
    reveal(&mut scene, key.index(49));
    assert_content_drawn(&scene, &mut canvas);
    tap(&mut scene, Key::new("nav"));
    assert_eq!(scene.tray_menu.as_ref(), Some(&root));
    tap(&mut scene, Key::new("nav"));
    assert_eq!(scene.page, Page::Panel(Panel::Tray));
    assert!(scene.tray_menu.is_none());
}

#[test]
fn a_scrolled_bluetooth_pairing_animation_cannot_cover_navigation() {
    let mut scene = scene(MenuTree::default());
    let bt = &mut scene.state_mut().bluetooth;
    let first = bt.devices[0].address.clone();
    for i in 0..12 {
        let mut device = bt.nearby[0].clone();
        device.address = format!("00:01:02:03:04:{i:02x}");
        bt.nearby.push(device);
    }
    scene.command(Command::OpenPanel(Panel::Bluetooth));
    scene
        .bt
        .pending
        .insert(first.clone(), (BtOp::Disconnect, scene.now));
    scene.update(scene.now, scene.utc);
    let mut before = Canvas::new(T1_WIDTH, T1_HEIGHT);
    let mut after = Canvas::new(T1_WIDTH, T1_HEIGHT);
    scene.render(&mut before);
    let device = scene.node_rect(Key::new("p/bt/dev").with(&first)).unwrap();
    let nav = scene.node_rect(Key::new("nav")).unwrap();
    scene.set_scroll_offset(device.cx() - nav.cx());
    assert!((device.cx() - scene.scroll.offset - nav.cx()).abs() < 0.01);
    scene.render(&mut after);
    let width = before.width() as usize;
    let rows_before = before.pixmap().pixels().chunks_exact(width);
    let rows_after = after.pixmap().pixels().chunks_exact(width);
    let pinned = scene.scroll.x0.floor() as usize;
    assert!(
        rows_before
            .zip(rows_after)
            .all(|(a, b)| a[..pinned] == b[..pinned])
    );
}

#[test]
fn hardware_submenus_follow_their_available_actions() {
    let tree = MenuTree::from_jsonc(
        r#"{
        "hardware":{"label":"Hardware"},
        "hardware.gpu":{"label":"GPU"},
        "hardware.gpu.switch":{"action":"switch","when":"hybrid_gpu"},
        "hardware.link":{"target":"hardware.gpu"},
        "hardware.display":{"action":"display","disabled":"fixed_brightness"},
        "hardware.font":{"provider":"fonts"}
    }"#,
    )
    .unwrap();
    let mut scene = scene(tree);
    scene.command(Command::OpenMenu(Some("hardware".into())));
    let actions = scene.take_actions();
    let Some(Action::EvaluateGuards { guards, .. }) = actions
        .iter()
        .find(|a| matches!(a, Action::EvaluateGuards { .. }))
    else {
        panic!("missing hardware probes")
    };
    assert_eq!(guards.len(), 2);
    assert!(
        guards
            .iter()
            .any(|g| g.id == "hardware.gpu.switch" && g.kind == GuardKind::When)
    );
    scene.apply_guards(
        "hardware",
        vec![
            ("hardware.gpu.switch".into(), GuardKind::When, false),
            ("hardware.display".into(), GuardKind::Disabled, true),
        ],
    );
    scene.update(scene.now, scene.utc);
    let key = |id| Key::new("m").with(id);
    assert!(scene.node_rect(key("hardware.gpu")).is_none());
    assert!(scene.node_rect(key("hardware.link")).is_none());
    assert!(scene.node_rect(key("hardware.font")).is_some());
    let display = &scene.animator.get(key("hardware.display")).unwrap().node;
    assert!(display.hit.is_none());
    assert!(display.parts.iter().any(
        |p| matches!(p, crate::node::Part::Text {text, ..} if text == crate::build::glyph::CHECK)
    ));
    scene.apply_guards(
        "hardware",
        vec![("hardware.gpu.switch".into(), GuardKind::When, true)],
    );
    scene.update(scene.now, scene.utc);
    tap(&mut scene, key("hardware.link"));
    assert_eq!(scene.page, Page::Menu("hardware.gpu".into()));
}

#[test]
fn render_every_installed_omarchy_submenu() {
    if !crate::paths::omarchy_root()
        .join("default/omarchy/omarchy-menu.jsonc")
        .is_file()
    {
        return; // The standalone renderer also builds on machines without Omarchy.
    }
    let tree = MenuTree::load();
    assert!(
        tree.children("").next().is_some(),
        "installed menu has no root entries"
    );
    let mut scene = scene(tree.clone());
    let mut canvas = Canvas::new(T1_WIDTH, T1_HEIGHT);
    let mut parents = vec![String::new()];
    let mut visited = std::collections::HashSet::new();
    while let Some(id) = parents.pop() {
        if !visited.insert(id.clone()) {
            continue;
        }
        scene.command(Command::OpenMenu(Some(id.clone())));
        // Include every potentially visible row, independent of host guards.
        let children: Vec<_> = tree
            .children(&id)
            .filter(|i| tree.is_visible(i, |_| true))
            .collect();
        let mut keys: Vec<_> = children.iter().map(|i| Key::new("m").with(&i.id)).collect();
        for action in scene.take_actions() {
            if let Action::LoadProvider { menu, provider } = action {
                let rows = menu::sample_provider_rows(&provider);
                keys.extend((0..rows.len()).map(|i| Key::new("m").with(&format!("{menu}.~{i}"))));
                scene.apply_provider(&menu, &provider, rows);
            }
        }
        scene.update(scene.now, scene.utc);
        assert_eq!(scene.page, Page::Menu(id.clone()));
        if !keys.is_empty() {
            assert_content_drawn(&scene, &mut canvas);
            for key in keys {
                reveal(&mut scene, key);
            }
            assert_content_drawn(&scene, &mut canvas);
        }
        for item in children.into_iter().filter(|i| tree.is_submenu(i)) {
            parents.push(item.target.as_ref().unwrap_or(&item.id).clone());
        }
    }
}
