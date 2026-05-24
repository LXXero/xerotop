//! Taskbar via the wlr-foreign-toplevel-management protocol. A dedicated thread
//! runs a calloop event loop driving a wayland-client queue (second compositor
//! connection) plus an activate channel. It tracks open toplevels and streams
//! sorted snapshots to the GTK thread; clicks come back as activate requests.

use calloop::EventLoop;
use calloop_wayland_source::WaylandSource;
use std::collections::HashMap;
use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self as handle, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self as mgr, ZwlrForeignToplevelManagerV1},
};

#[derive(Clone, Default)]
pub struct Toplevel {
    pub id: u64,
    pub title: String,
    pub app_id: String,
    pub activated: bool,
    pub minimized: bool,
}

/// Click actions sent from the GTK thread back to the wayland thread.
pub enum Action {
    Activate(u64),
    ToggleMinimize(u64),
}

struct State {
    toplevels: HashMap<ObjectId, Toplevel>,
    handles: HashMap<u64, ZwlrForeignToplevelHandleV1>,
    next_id: u64,
    seat: Option<WlSeat>,
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

    fn activate(&self, id: u64) {
        if let (Some(h), Some(seat)) = (self.handles.get(&id), self.seat.as_ref()) {
            h.activate(seat);
        }
    }

    fn toggle_minimize(&self, id: u64) {
        let Some(h) = self.handles.get(&id) else {
            return;
        };
        let minimized = self
            .toplevels
            .values()
            .find(|t| t.id == id)
            .is_some_and(|t| t.minimized);
        if minimized {
            h.unset_minimized();
        } else {
            h.set_minimized();
        }
    }
}

/// Spawn the wayland listener; returns (snapshot receiver, activate sender).
pub fn spawn() -> (
    async_channel::Receiver<Vec<Toplevel>>,
    calloop::channel::Sender<Action>,
) {
    let (tx, rx) = async_channel::unbounded();
    let (atx, achan) = calloop::channel::channel::<Action>();
    std::thread::spawn(move || {
        if let Err(e) = run(tx, achan) {
            eprintln!("xerotop: taskbar wayland thread exited: {e}");
        }
    });
    (rx, atx)
}

fn run(
    tx: async_channel::Sender<Vec<Toplevel>>,
    achan: calloop::channel::Channel<Action>,
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, queue) = registry_queue_init::<State>(&conn)?;
    let qh = queue.handle();
    let _mgr: ZwlrForeignToplevelManagerV1 = globals.bind(&qh, 1..=3, ())?;
    let seat: Option<WlSeat> = globals.bind(&qh, 1..=8, ()).ok();

    let mut state = State {
        toplevels: HashMap::new(),
        handles: HashMap::new(),
        next_id: 0,
        seat,
        tx,
        dirty: false,
    };

    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let handle = event_loop.handle();
    WaylandSource::new(conn, queue)
        .insert(handle.clone())
        .map_err(|_| "failed to insert wayland source")?;
    handle
        .insert_source(achan, |event, _, state| {
            if let calloop::channel::Event::Msg(action) = event {
                match action {
                    Action::Activate(id) => state.activate(id),
                    Action::ToggleMinimize(id) => state.toggle_minimize(id),
                }
            }
        })
        .map_err(|_| "failed to insert activate channel")?;

    event_loop.run(None, &mut state, |state| state.flush())?;
    Ok(())
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

impl Dispatch<WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: wayland_client::protocol::wl_seat::Event,
        _: &(),
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
            let id = state.next_id;
            state.next_id += 1;
            state.toplevels.insert(
                toplevel.id(),
                Toplevel {
                    id,
                    ..Default::default()
                },
            );
            state.handles.insert(id, toplevel);
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
        let oid = proxy.id();
        match event {
            handle::Event::Title { title } => {
                if let Some(t) = this.toplevels.get_mut(&oid) {
                    t.title = title;
                }
            }
            handle::Event::AppId { app_id } => {
                if let Some(t) = this.toplevels.get_mut(&oid) {
                    t.app_id = app_id;
                }
            }
            handle::Event::State { state } => {
                let vals: Vec<u32> = state
                    .chunks_exact(4)
                    .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                if let Some(t) = this.toplevels.get_mut(&oid) {
                    t.activated = vals.contains(&(handle::State::Activated as u32));
                    t.minimized = vals.contains(&(handle::State::Minimized as u32));
                }
            }
            handle::Event::Done => {
                this.dirty = true;
            }
            handle::Event::Closed => {
                if let Some(t) = this.toplevels.remove(&oid) {
                    this.handles.remove(&t.id);
                }
                this.dirty = true;
            }
            _ => {}
        }
    }
}
