//! Taskbar via the wlr-foreign-toplevel-management protocol. A dedicated thread
//! runs a wayland-client event loop (a second connection to the compositor),
//! tracks open toplevels, and streams sorted snapshots to the GTK thread over an
//! async channel. The GTK side rebuilds the list dynamically (no pre-allocated
//! slots — the whole reason we left ewwii's static config).

use std::collections::HashMap;
use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self as handle, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self as mgr, ZwlrForeignToplevelManagerV1},
};

#[derive(Clone, Default)]
pub struct Toplevel {
    pub title: String,
    pub app_id: String,
    pub activated: bool,
}

struct State {
    toplevels: HashMap<ObjectId, Toplevel>,
    tx: async_channel::Sender<Vec<Toplevel>>,
    dirty: bool,
}

impl State {
    fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let mut list: Vec<Toplevel> = self.toplevels.values().cloned().collect();
        list.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        let _ = self.tx.send_blocking(list);
    }
}

/// Spawn the wayland listener thread; returns a receiver of window snapshots.
pub fn spawn() -> async_channel::Receiver<Vec<Toplevel>> {
    let (tx, rx) = async_channel::unbounded();
    std::thread::spawn(move || {
        if let Err(e) = run(tx) {
            eprintln!("xerotop: taskbar wayland thread exited: {e}");
        }
    });
    rx
}

fn run(tx: async_channel::Sender<Vec<Toplevel>>) -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
    let qh = queue.handle();
    // bind the toplevel manager (errors if the compositor lacks the protocol)
    let _mgr: ZwlrForeignToplevelManagerV1 = globals.bind(&qh, 1..=3, ())?;

    let mut state = State {
        toplevels: HashMap::new(),
        tx,
        dirty: false,
    };
    loop {
        queue.blocking_dispatch(&mut state)?;
        state.flush();
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: mgr::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let mgr::Event::Toplevel { toplevel } = event {
            state.toplevels.insert(toplevel.id(), Toplevel::default());
        }
    }

    wayland_client::event_created_child!(State, ZwlrForeignToplevelManagerV1, [
        mgr::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(
        this: &mut Self,
        proxy: &ZwlrForeignToplevelHandleV1,
        event: handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = proxy.id();
        match event {
            handle::Event::Title { title } => {
                if let Some(t) = this.toplevels.get_mut(&id) {
                    t.title = title;
                }
            }
            handle::Event::AppId { app_id } => {
                if let Some(t) = this.toplevels.get_mut(&id) {
                    t.app_id = app_id;
                }
            }
            handle::Event::State { state } => {
                let activated = state
                    .chunks_exact(4)
                    .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
                    .any(|v| v == handle::State::Activated as u32);
                if let Some(t) = this.toplevels.get_mut(&id) {
                    t.activated = activated;
                }
            }
            handle::Event::Done => {
                this.dirty = true;
            }
            handle::Event::Closed => {
                this.toplevels.remove(&id);
                this.dirty = true;
            }
            _ => {}
        }
    }
}
