//! The native participant `tilrd` (design T1): its method registry, the handle to
//! the supervisor and the supervisor's hooks for the bus. Owner: `bus-broker-core`.
//!
//! Each namespace of the core is a file of its own with one
//! `pub fn register(registry: &mut Registry, core: &Core)`; [`registry`] calls them
//! all, so a lane adds methods in its file without editing this one. A feature
//! turned off registers nothing (B14): its methods are `not_found`, as is every
//! method of a reserved namespace nobody registered yet.
//!
//! A method reaches the supervisor's state with [`SupervisorHandle::ask`] (or
//! [`SupervisorHandle::with`]): the closure runs on the supervisor's own task, with
//! `&mut Supervisor`. Since this module lives inside `supervisor` (see `bus/mod.rs`),
//! the supervisor's private fields and paths (create, input with acknowledgement,
//! resize, close, frames, files, exec, clipboard) are at hand. Its own state goes in a
//! slot of [`CoreState`] ([`CoreState::slot`], [`CoreState::slot_ref`]). What the
//! supervisor sees happen to panes reaches the bus through [`BusObserver`]s, added
//! with [`Registry::observe`] and called with `&Supervisor` on its task; the call
//! sites are in `supervisor.rs` ([`pane_output`], [`pane_event`]), built only while
//! an observer listens.
//!
//! Rules for the lanes (also in `bus/mod.rs`):
//! - helpers are free functions in the lane's file, `fn x(s: &mut Supervisor, …)`,
//!   not `impl Supervisor` blocks (methods of parallel lanes would collide);
//! - a `match` on [`CoreCmd`] or [`PaneEvent`] outside this file ends with `_ => …`;
//!   a lane that needs a typed command appends a variant at the end of [`CoreCmd`]
//!   and its arm in [`Supervisor::on_core`];
//! - `broker.rs` and `transport.rs` are not edited by the lanes of this module.

/// `ssh_agent.*`, called on clients (`bus-core-services`).
pub mod agent;
/// `clipboard.*` (`bus-core-services`).
pub mod clipboard;
/// What `server`, `fs`, `exec` and `clipboard` share (`bus-core-services`).
pub mod common;
/// `exec.*` (`bus-core-services`).
pub mod exec;
/// `fs.*` (`bus-core-services`).
pub mod fs;
/// `host.open`, called on clients (`bus-core-services`).
pub mod host;
/// The JSON mapping of the core's payloads (`bus-core-panes`).
pub mod json;
/// The pane rights of T7 (`bus-pane-rights`).
pub mod pane_rights;
/// `pane.*` (`bus-core-panes`).
pub mod panes;
/// `pane.profiles`, `pane.create {profile}` (`pane-profiles`).
pub mod profiles;
/// `server.*` (`bus-core-services`).
pub mod server;

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use tilr_bus::BusError;
use tokio::sync::{mpsc, oneshot};

use super::broker::{self, BrokerHandle};
use super::streams::NativeMethod;
use crate::link::FrameHead;
use crate::supervisor::{Event, PaneId, Supervisor};

/// What a namespace's `register` is given.
#[derive(Clone)]
pub struct Core {
    pub supervisor: SupervisorHandle,
    pub broker: BrokerHandle,
    pub features: Features,
}

/// The features a server can be run without (B14), as `server.info.features` names
/// them: a namespace whose feature is off registers nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Features {
    pub clipboard: bool,
    pub files: bool,
    pub host_events: bool,
    pub agent: bool,
    pub exec: bool,
}

/// The native participant's methods and the supervisor's observers.
#[derive(Default)]
pub struct Registry {
    methods: Vec<NativeMethod>,
    observers: Vec<Arc<dyn BusObserver>>,
}

impl Registry {
    /// Provide a method as `tilrd`. A name added twice is a bug: the later wins,
    /// with a log line.
    pub fn add(&mut self, method: NativeMethod) {
        if let Some(i) = self
            .methods
            .iter()
            .position(|m| m.spec.method == method.spec.method)
        {
            tracing::error!(target: "tilrd::bus", method = method.spec.method, "registered twice");
            self.methods.remove(i);
        }
        self.methods.push(method);
    }

    /// Have the supervisor tell `observer` what happens to panes.
    pub fn observe(&mut self, observer: Arc<dyn BusObserver>) {
        self.observers.push(observer);
    }

    pub fn methods(&self) -> impl Iterator<Item = &NativeMethod> {
        self.methods.iter()
    }

    pub fn into_parts(self) -> (Vec<NativeMethod>, Vec<Arc<dyn BusObserver>>) {
        (self.methods, self.observers)
    }
}

/// The core's registry: every namespace's methods, in one place.
pub fn registry(core: &Core) -> Registry {
    let mut registry = Registry::default();
    registry.add(broker::ping());
    super::control::register(&mut registry, core);
    super::acl::register(&mut registry, core);
    panes::register(&mut registry, core);
    profiles::register(&mut registry, core);
    pane_rights::register(&mut registry, core);
    server::register(&mut registry, core);
    fs::register(&mut registry, core);
    exec::register(&mut registry, core);
    clipboard::register(&mut registry, core);
    host::register(&mut registry, core);
    agent::register(&mut registry, core);
    registry
}

/// A request for the supervisor's task. `With` covers what the core's methods
/// need; a lane that needs a typed request appends a variant at the end, with its
/// arm in [`Supervisor::on_core`].
pub enum CoreCmd {
    /// Run this closure on the supervisor's task.
    With(Box<dyn FnOnce(&mut Supervisor) + Send>),
}

/// The way from the bus to the supervisor's task: commands go through its event
/// queue, in the order sent. Clones share it.
#[derive(Clone)]
pub struct SupervisorHandle(mpsc::UnboundedSender<Event>);

impl SupervisorHandle {
    pub(in crate::supervisor) fn new(events: mpsc::UnboundedSender<Event>) -> Self {
        Self(events)
    }

    /// Queue a command; `unavailable` once the supervisor is gone.
    pub fn send(&self, cmd: CoreCmd) -> Result<(), BusError> {
        self.0
            .send(Event::Core(cmd))
            .map_err(|_| BusError::unavailable("tilrd is shutting down"))
    }

    /// Run `f` on the supervisor's task, without waiting.
    pub fn with(&self, f: impl FnOnce(&mut Supervisor) + Send + 'static) -> Result<(), BusError> {
        self.send(CoreCmd::With(Box::new(f)))
    }

    /// Run `f` on the supervisor's task and wait for what it returns. The command
    /// is queued when this is called, not when the future is first polled: a
    /// handler that calls it before returning its future keeps the call's order.
    pub fn ask<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Supervisor) -> R + Send + 'static,
    ) -> impl Future<Output = Result<R, BusError>> + Send + 'static {
        let (tx, rx) = oneshot::channel();
        let sent = self.with(move |supervisor| {
            let _ = tx.send(f(supervisor));
        });
        async move {
            sent?;
            rx.await
                .map_err(|_| BusError::unavailable("tilrd is shutting down"))
        }
    }
}

/// What the supervisor saw happen to a pane, for the bus.
#[derive(Clone, Debug)]
pub enum PaneEvent {
    /// The pane's shell runs; the pane is listed from now on.
    Created { pane: PaneId },
    /// Title or working directory changed.
    Changed { pane: PaneId },
    /// A bell or a finished command (`PaneActivity`).
    Activity {
        pane: PaneId,
        activity: tilr_proto::PaneActivity,
    },
    /// The pane process is gone; `exit_code` -1 when it died without telling.
    Exited { pane: PaneId, exit_code: i32 },
}

/// Told by the supervisor, on its task, what happens to panes. Keep it short: no
/// blocking, no waiting; queue what takes longer.
pub trait BusObserver: Send + Sync {
    /// A frame of `pane` (the pane process's packet, encoded and compressed once,
    /// as `tilr.v1` subscribers get it). `supervisor` is read as it is now.
    fn pane_output(
        &self,
        _supervisor: &Supervisor,
        _pane: &PaneId,
        _head: FrameHead,
        _packet: &Bytes,
    ) {
    }

    /// Something happened to a pane; `supervisor` holds the pane's state after it
    /// (gone for `Exited`).
    fn pane_event(&self, _supervisor: &Supervisor, _event: &PaneEvent) {}
}

/// What the supervisor keeps for the bus (its field `bus`).
#[derive(Default)]
pub struct CoreState {
    observers: Vec<Arc<dyn BusObserver>>,
    slots: HashMap<TypeId, Box<dyn Any + Send>>,
    broker: Option<BrokerHandle>,
}

impl CoreState {
    pub fn new(broker: BrokerHandle, observers: Vec<Arc<dyn BusObserver>>) -> Self {
        Self {
            observers,
            slots: HashMap::new(),
            broker: Some(broker),
        }
    }

    /// The broker; `None` in the supervisor's own unit tests.
    pub fn broker(&self) -> Option<&BrokerHandle> {
        self.broker.as_ref()
    }

    /// A lane's own state on the supervisor, made on first use.
    pub fn slot<T: Default + Send + 'static>(&mut self) -> &mut T {
        self.slots
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(T::default()))
            .downcast_mut::<T>()
            .expect("a slot holds its own type")
    }

    /// A lane's slot, read-only (an observer's view); `None` before first use.
    pub fn slot_ref<T: Send + 'static>(&self) -> Option<&T> {
        self.slots.get(&TypeId::of::<T>())?.downcast_ref::<T>()
    }

    /// Whether any observer listens: the call sites build nothing otherwise.
    pub fn observing(&self) -> bool {
        !self.observers.is_empty()
    }
}

/// Hook: a frame of a pane (`Supervisor::on_pane_output`), for the observers.
pub(crate) fn pane_output(
    supervisor: &Supervisor,
    pane: &PaneId,
    head: Option<FrameHead>,
    packet: &Bytes,
) {
    let Some(head) = head else {
        return;
    };
    for observer in &supervisor.bus.observers {
        observer.pane_output(supervisor, pane, head, packet);
    }
}

/// Hook: something happened to a pane, for the observers.
pub(crate) fn pane_event(supervisor: &Supervisor, event: &PaneEvent) {
    for observer in &supervisor.bus.observers {
        observer.pane_event(supervisor, event);
    }
}

impl Supervisor {
    /// A command of the bus, on the supervisor's task.
    pub(crate) fn on_core(&mut self, cmd: CoreCmd) {
        match cmd {
            CoreCmd::With(f) => f(self),
        }
    }
}
