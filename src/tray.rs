//! System tray via StatusNotifier (the `system-tray` crate, same host
//! `wlr-trayd` used). A dedicated thread runs a tokio runtime + the tray host,
//! streams item snapshots (incl. their DBus menu) to GTK, and takes click and
//! menu-click requests back.

use std::collections::HashMap;
use std::sync::Arc;
use system_tray::client::{ActivateRequest, Client, Event, UpdateEvent};
use system_tray::item::{IconPixmap, StatusNotifierItem};
use system_tray::menu::{MenuItem, MenuType, TrayMenu};

#[derive(Clone, PartialEq)]
pub struct MenuEntry {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
    pub children: Vec<MenuEntry>,
}

#[derive(Clone, Default)]
pub struct TrayItem {
    pub id: String, // service address
    pub title: String,
    pub icon_name: Option<String>,
    pub icon_theme_path: Option<String>,
    pub pixmap: Option<(i32, i32, Vec<u8>)>, // (w, h, ARGB32)
    pub menu_path: Option<String>,
    pub menu: Vec<MenuEntry>,
}

/// Requests sent from the GTK thread back to the tray host.
pub enum TrayAction {
    Activate(String),                 // address (left-click)
    MenuClick(String, String, i32),   // address, menu_path, submenu_id
    AboutToShow(String, String, i32), // address, menu_path, menu/submenu id
}

fn best_pixmap(pix: Option<&Vec<IconPixmap>>) -> Option<(i32, i32, Vec<u8>)> {
    let p = pix?.iter().max_by_key(|p| p.width * p.height)?;
    Some((p.width, p.height, p.pixels.clone()))
}

fn build_entries(items: &[MenuItem]) -> Vec<MenuEntry> {
    items
        .iter()
        .filter(|m| m.visible)
        .map(|m| MenuEntry {
            id: m.id,
            label: m.label.clone().unwrap_or_default(),
            enabled: m.enabled,
            separator: matches!(m.menu_type, MenuType::Separator),
            children: build_entries(&m.submenu),
        })
        .collect()
}

fn menu_entries(menu: &TrayMenu) -> Vec<MenuEntry> {
    build_entries(&menu.submenus)
}

fn to_item(addr: &str, item: &StatusNotifierItem) -> TrayItem {
    TrayItem {
        id: addr.to_string(),
        title: item.title.clone().unwrap_or_default(),
        icon_name: item.icon_name.clone().filter(|s| !s.is_empty()),
        icon_theme_path: item.icon_theme_path.clone(),
        pixmap: best_pixmap(item.icon_pixmap.as_ref()),
        menu_path: item.menu.clone(),
        menu: Vec::new(),
    }
}

fn snapshot(items: &HashMap<String, TrayItem>) -> Vec<TrayItem> {
    let mut v: Vec<TrayItem> = items.values().cloned().collect();
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
}

/// Spawn the tray host thread; returns (snapshot receiver, action sender).
pub fn spawn() -> (
    async_channel::Receiver<Vec<TrayItem>>,
    async_channel::Sender<TrayAction>,
) {
    let (tx, rx) = async_channel::unbounded();
    let (atx, arx) = async_channel::unbounded::<TrayAction>();
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("xerotop: tray runtime: {e}");
                return;
            }
        };
        rt.block_on(async move {
            if let Err(e) = run(tx, arx).await {
                eprintln!("xerotop: tray host exited: {e}");
            }
        });
    });
    (rx, atx)
}

async fn run(
    tx: async_channel::Sender<Vec<TrayItem>>,
    arx: async_channel::Receiver<TrayAction>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(Client::new().await?);
    let mut sub = client.subscribe();
    let mut items: HashMap<String, TrayItem> = HashMap::new();

    {
        let initial = client.items();
        if let Ok(guard) = initial.lock() {
            for (addr, (item, menu)) in guard.iter() {
                let mut it = to_item(addr, item);
                if let Some(m) = menu {
                    it.menu = menu_entries(m);
                }
                items.insert(addr.clone(), it);
            }
        }
    }
    let _ = tx.send(snapshot(&items)).await;

    loop {
        tokio::select! {
            ev = sub.recv() => {
                // Only push a fresh snapshot (= full tray re-render in GTK) for
                // events that actually changed something we display.
                let mut changed = true;
                match ev {
                    Ok(Event::Add(addr, item)) => {
                        items.insert(addr.clone(), to_item(&addr, &item));
                        // Poke AboutToShow(root) so apps that build their menu
                        // lazily populate it now; the refreshed layout comes back
                        // as a later UpdateEvent::Menu.
                        if let Some(mp) = item.menu.clone() {
                            let c = client.clone();
                            let a = addr.clone();
                            tokio::spawn(async move {
                                let _ = c.about_to_show_menuitem(a, mp, 0).await;
                            });
                        }
                    }
                    Ok(Event::Update(addr, update)) => {
                        if let Some(it) = items.get_mut(&addr) {
                            match update {
                                UpdateEvent::Icon { icon_name, icon_pixmap } => {
                                    it.icon_name = icon_name.filter(|s| !s.is_empty());
                                    it.pixmap = best_pixmap(icon_pixmap.as_ref());
                                }
                                UpdateEvent::Title(t) => it.title = t.unwrap_or_default(),
                                UpdateEvent::Menu(menu) => it.menu = menu_entries(&menu),
                                _ => changed = false,
                            }
                        } else {
                            changed = false;
                        }
                    }
                    Ok(Event::Remove(addr)) => { items.remove(&addr); }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
                if changed {
                    let _ = tx.send(snapshot(&items)).await;
                }
            }
            req = arx.recv() => {
                match req {
                    // Run D-Bus calls off the loop so they never block processing
                    // of incoming menu refreshes (which would let ids go stale).
                    Ok(TrayAction::Activate(address)) => {
                        let c = client.clone();
                        tokio::spawn(async move {
                            let _ = c
                                .activate(ActivateRequest::Default { address, x: 0, y: 0 })
                                .await;
                        });
                    }
                    Ok(TrayAction::MenuClick(address, menu_path, submenu_id)) => {
                        let c = client.clone();
                        tokio::spawn(async move {
                            if let Err(e) = c
                                .activate(ActivateRequest::MenuItem {
                                    address,
                                    menu_path,
                                    submenu_id,
                                })
                                .await
                            {
                                eprintln!("xerotop: menu activate failed: {e}");
                            }
                        });
                    }
                    Ok(TrayAction::AboutToShow(address, menu_path, id)) => {
                        let c = client.clone();
                        tokio::spawn(async move {
                            let _ = c.about_to_show_menuitem(address, menu_path, id).await;
                        });
                    }
                    Err(_) => break,
                }
            }
        }
    }
    Ok(())
}
