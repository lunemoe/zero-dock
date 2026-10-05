//! PulseAudio subscriptions, process matching, mute and volume. Ported from
//! `src/audio.c`.
//!
//! The C implementation expressed its connection state through six loose
//! flags (`querying`, `again`, `closing`, `query_op`, `query_id`,
//! `reconnect_id`) that could describe illegal combinations, and guarded
//! against stale server callbacks with pointer compares. Here the same
//! invariants are carried by ownership:
//!
//! * the `Audio` state lives in its own `Rc<RefCell<..>>` inside the dock, so
//!   PulseAudio callbacks (which may fire synchronously from libpulse while
//!   the dock is borrowed) mutate only audio state;
//! * every context callback captures a generation counter bumped on each
//!   reconnect, so a callback from a replaced connection can never touch the
//!   new connection's buffers;
//! * borrowed-state re-entrancy defers the transition through an idle
//!   callback instead of dropping it.

use crate::dock::Dock;
use crate::util::{DockWeak, Timer};
use libpulse_binding as pa;
use libpulse_binding::callbacks::ListResult;
use libpulse_binding::context::introspect::SinkInputInfo;
use libpulse_binding::context::subscribe::Facility;
use libpulse_binding::context::subscribe::InterestMaskSet;
use libpulse_binding::context::Context as PaContext;
use libpulse_binding::context::State;
use libpulse_binding::volume::{ChannelVolumes, Volume};
use libpulse_glib_binding::Mainloop;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

const PA_VOLUME_NORM: u32 = 65536;

type InfoOp = pa::operation::Operation<dyn FnMut(ListResult<&SinkInputInfo>)>;

#[derive(Clone, Debug)]
#[allow(dead_code)] // `name` mirrors the C data model; unused for now
pub struct Stream {
    pub index: u32,
    pub pid: i32,
    pub binary: Option<String>,
    pub name: Option<String>,
    pub mute: bool,
    pub corked: bool,
    pub volume: ChannelVolumes,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioStatus {
    pub present: bool,
    pub muted: bool,
    pub playing: bool,
    pub percent: u32,
}

pub struct Audio {
    pub dock: DockWeak,
    pub mainloop: Mainloop,
    pub ctx: Option<PaContext>,
    pub generation: u64,
    pub streams: Vec<Stream>,
    pub pending: Option<Vec<Stream>>,
    pub query_op: Option<InfoOp>,
    pub querying: bool,
    pub again: bool,
    pub closing: bool,
    pub query_id: Timer,
    pub reconnect_id: Timer,
    /// Weak handle to this Audio inside its own `Rc`, captured by callbacks.
    pub weak: AudioWeak,
}

pub type AudioRef = Rc<RefCell<Audio>>;
pub type AudioWeak = Weak<RefCell<Audio>>;

impl Drop for Audio {
    fn drop(&mut self) {
        self.closing = true;
        self.query_id.clear();
        self.reconnect_id.clear();
        self.query_abort();
        if let Some(mut ctx) = self.ctx.take() {
            ctx.set_state_callback(None);
            ctx.set_subscribe_callback(None);
            ctx.disconnect();
        }
    }
}

impl Audio {
    pub fn new(dock: DockWeak) -> Option<AudioRef> {
        let mainloop = Mainloop::new(None)?;
        let rc: AudioRef = Rc::new_cyclic(|weak: &AudioWeak| {
            RefCell::new(Audio {
                dock,
                mainloop,
                ctx: None,
                generation: 0,
                streams: Vec::new(),
                pending: None,
                query_op: None,
                querying: false,
                again: false,
                closing: false,
                query_id: Timer::default(),
                reconnect_id: Timer::default(),
                weak: weak.clone(),
            })
        });
        rc.borrow_mut().connect();
        Some(rc)
    }

    fn connect(&mut self) {
        // Drop any previous context. Callbacks must be detached before the
        // disconnect so no stale transition reaches the state machine.
        self.query_abort();
        if let Some(mut ctx) = self.ctx.take() {
            ctx.set_state_callback(None);
            ctx.set_subscribe_callback(None);
            ctx.disconnect();
        }
        let Some(mut ctx) = PaContext::new(&self.mainloop, "Zero Dock") else {
            self.schedule_reconnect();
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        let weak = self.weak.clone();
        ctx.set_state_callback(Some(Box::new(move || {
            state_changed(&weak, generation);
        })));
        self.ctx = Some(ctx);
        let connect_result =
            self.ctx
                .as_mut()
                .unwrap()
                .connect(None, pa::context::FlagSet::NOAUTOSPAWN, None);
        if connect_result.is_err() {
            self.schedule_reconnect();
        }
    }

    fn schedule_reconnect(&mut self) {
        if !self.reconnect_id.is_set() {
            let weak = self.weak.clone();
            self.reconnect_id.set_timeout_seconds(3, move || {
                if let Some(rc) = weak.upgrade() {
                    if let Ok(mut audio) = rc.try_borrow_mut() {
                        if !audio.closing {
                            audio.reconnect_id.take();
                            audio.connect();
                        }
                    }
                }
                glib::ControlFlow::Break
            });
        }
    }

    fn query_abort(&mut self) {
        self.querying = false;
        self.again = false;
        self.pending = None;
        if let Some(mut op) = self.query_op.take() {
            // pa_operation_cancel + unref, like the C teardown.
            op.cancel();
        }
    }

    fn query_schedule(&mut self) {
        if self.closing {
            return;
        }
        if !self.query_id.is_set() {
            let weak = self.weak.clone();
            self.query_id.set_timeout(35, move || {
                query_run(&weak);
                glib::ControlFlow::Break
            });
        }
    }
}

fn state_changed(weak: &AudioWeak, generation: u64) {
    let Some(rc) = weak.upgrade() else { return };
    enum Next {
        None,
        ScheduleQuery,
        DisconnectCleanup,
    }
    let next = {
        let mut this = match rc.try_borrow_mut() {
            Ok(borrowed) => borrowed,
            // Called synchronously from libpulse while the audio state is
            // borrowed (e.g. connect() reporting immediate failure): replay
            // the transition from the main loop instead of dropping it.
            Err(_) => {
                let weak = weak.clone();
                glib::idle_add_local_once(move || state_changed(&weak, generation));
                return;
            }
        };
        if this.closing || this.generation != generation {
            return;
        }
        let Some(ctx) = this.ctx.as_mut() else { return };
        match ctx.get_state() {
            State::Ready => {
                let weak2 = weak.clone();
                ctx.set_subscribe_callback(Some(Box::new(
                    move |facility: Option<Facility>, _operation, _index: u32| {
                        if facility == Some(Facility::SinkInput) {
                            if let Some(rc) = weak2.upgrade() {
                                if let Ok(mut audio) = rc.try_borrow_mut() {
                                    audio.query_schedule();
                                }
                            }
                        }
                    },
                )));
                // Dropping the operation only unrefs it, the server-side
                // subscription stays active (same as pa_operation_unref in C).
                let op = ctx.subscribe(InterestMaskSet::SINK_INPUT, |_success| {});
                drop(op);
                Next::ScheduleQuery
            }
            State::Failed | State::Terminated => {
                this.query_abort();
                this.streams.clear();
                Next::DisconnectCleanup
            }
            _ => Next::None,
        }
    };
    match next {
        Next::ScheduleQuery => {
            if let Some(rc) = weak.upgrade() {
                if let Ok(mut audio) = rc.try_borrow_mut() {
                    audio.query_schedule();
                }
            }
        }
        Next::DisconnectCleanup => {
            update_audio_buttons(weak);
            if let Some(rc) = weak.upgrade() {
                if let Ok(mut audio) = rc.try_borrow_mut() {
                    audio.schedule_reconnect();
                }
            }
        }
        Next::None => {}
    }
}

fn query_run(weak: &AudioWeak) {
    let Some(rc) = weak.upgrade() else { return };
    let mut this = match rc.try_borrow_mut() {
        Ok(borrowed) => borrowed,
        Err(_) => return,
    };
    this.query_id.take();
    if this.closing {
        return;
    }
    let ready = this
        .ctx
        .as_ref()
        .is_some_and(|ctx| ctx.get_state() == State::Ready);
    if !ready {
        return;
    }
    if this.querying {
        this.again = true;
        return;
    }
    this.querying = true;
    this.pending = Some(Vec::new());
    let generation = this.generation;
    let weak2 = weak.clone();
    let introspector = this.ctx.as_ref().unwrap().introspect();
    let op = introspector.get_sink_input_info_list(move |result| {
        info_callback(&weak2, generation, result);
    });
    this.query_op = Some(op);
}

fn info_callback(weak: &AudioWeak, generation: u64, result: ListResult<&SinkInputInfo>) {
    let Some(rc) = weak.upgrade() else { return };
    let mut this = match rc.try_borrow_mut() {
        Ok(borrowed) => borrowed,
        Err(_) => return,
    };
    // A query may outlive a server disconnect. Never let callbacks from an
    // old context mutate the buffers belonging to the replacement connection.
    if this.closing || this.generation != generation || !this.querying || this.pending.is_none() {
        return;
    }
    match result {
        ListResult::Item(info) => {
            let pid = info
                .proplist
                .get_str("application.process.id")
                .and_then(|s| s.parse::<i32>().ok())
                .filter(|p| *p > 0);
            let binary = info.proplist.get_str("application.process.binary");
            let name = info.proplist.get_str("application.name");
            this.pending.as_mut().unwrap().push(Stream {
                index: info.index,
                pid: pid.unwrap_or(0),
                binary,
                name,
                mute: info.mute,
                corked: info.corked,
                volume: info.volume,
            });
        }
        ListResult::End => {
            this.querying = false;
            // Dropping the operation unrefs it without cancelling.
            this.query_op = None;
            this.streams = this.pending.take().unwrap();
            let again = this.again;
            this.again = false;
            drop(this);
            update_audio_buttons(weak);
            if again {
                if let Some(rc) = weak.upgrade() {
                    if let Ok(mut audio) = rc.try_borrow_mut() {
                        audio.query_schedule();
                    }
                }
            }
        }
        ListResult::Error => {
            this.querying = false;
            this.query_op = None;
            this.pending = None;
            let again = this.again;
            this.again = false;
            if again {
                if let Some(rc) = weak.upgrade() {
                    if let Ok(mut audio) = rc.try_borrow_mut() {
                        audio.query_schedule();
                    }
                }
            }
        }
    }
}

/// Push changed audio state into the buttons and preview UI.
fn update_audio_buttons(weak: &AudioWeak) {
    let dock = match weak.upgrade() {
        Some(rc) => match rc.try_borrow() {
            Ok(audio) => audio.dock.upgrade(),
            Err(_) => return,
        },
        None => return,
    };
    if let Some(dock) = dock {
        crate::util::with_dock(&dock, |d| d.update_audio_buttons());
    }
}

// ---------------------------------------------------------------------------
// Dock-facing API
// ---------------------------------------------------------------------------

impl Dock {
    pub fn audio_status(&mut self, button_id: u64) -> AudioStatus {
        let mut status = AudioStatus {
            muted: true,
            ..Default::default()
        };
        let Some(audio_rc) = self.audio.clone() else {
            return AudioStatus::default();
        };
        let audio = audio_rc.borrow();
        for s in &audio.streams {
            if self.stream_matches(button_id, s) {
                status.present = true;
                status.muted &= s.mute;
                status.playing |= !s.corked;
                let percent =
                    (100.0 * s.volume.avg().0 as f64 / PA_VOLUME_NORM as f64).round() as u32;
                status.percent = status.percent.max(percent);
            }
        }
        if !status.present {
            status.muted = false;
        }
        status
    }

    pub fn audio_mute(&mut self, button_id: u64) {
        let Some(audio_rc) = self.audio.clone() else {
            return;
        };
        let mute = !self.audio_status(button_id).muted;
        {
            let mut audio = match audio_rc.try_borrow_mut() {
                Ok(a) => a,
                Err(_) => return,
            };
            if audio.closing {
                return;
            }
            let ready = audio
                .ctx
                .as_ref()
                .is_some_and(|ctx| ctx.get_state() == State::Ready);
            if !ready {
                return;
            }
            let indexes: Vec<u32> = audio
                .streams
                .iter()
                .filter(|s| self.stream_matches(button_id, s))
                .map(|s| s.index)
                .collect();
            if indexes.is_empty() {
                return;
            }
            let Audio { ctx, streams, .. } = &mut *audio;
            let ctx = match ctx.as_mut() {
                Some(c) => c,
                None => return,
            };
            let mut introspector = ctx.introspect();
            for s in streams.iter_mut() {
                if indexes.contains(&s.index) {
                    // Unref-only drop: the server-side operation continues,
                    // matching pa_operation_unref in C.
                    let op = introspector.set_sink_input_mute(s.index, mute, None);
                    drop(op);
                    s.mute = mute;
                }
            }
        }
        self.update_audio_buttons();
    }

    pub fn audio_volume(&mut self, button_id: u64, steps: i32) {
        let Some(audio_rc) = self.audio.clone() else {
            return;
        };
        {
            let mut audio = match audio_rc.try_borrow_mut() {
                Ok(a) => a,
                Err(_) => return,
            };
            if audio.closing {
                return;
            }
            let ready = audio
                .ctx
                .as_ref()
                .is_some_and(|ctx| ctx.get_state() == State::Ready);
            if !ready {
                return;
            }
            let indexes: Vec<u32> = audio
                .streams
                .iter()
                .filter(|s| self.stream_matches(button_id, s))
                .map(|s| s.index)
                .collect();
            if indexes.is_empty() {
                return;
            }
            let Audio { ctx, streams, .. } = &mut *audio;
            let ctx = match ctx.as_mut() {
                Some(c) => c,
                None => return,
            };
            let mut introspector = ctx.introspect();
            for s in streams.iter_mut() {
                if !indexes.contains(&s.index) {
                    continue;
                }
                let old = s.volume.avg().0 as i64;
                let target = (old + steps as i64 * PA_VOLUME_NORM as i64 / 20)
                    .clamp(0, 2 * PA_VOLUME_NORM as i64);
                let mut volume: ChannelVolumes = s.volume;
                if volume.scale(Volume(target as u32)).is_some() {
                    // Unref-only drop: the server-side operation continues.
                    let op = introspector.set_sink_input_volume(s.index, &volume, None);
                    drop(op);
                    s.volume = volume;
                }
            }
        }
        self.update_audio_buttons();
    }

    /// Does an audio stream belong to this button? Prefer process ancestry,
    /// fall back to executable/class hints; a sibling button owning the
    /// stream's process excludes the match. Ported from `zd_stream_matches`.
    pub fn stream_matches(&mut self, button_id: u64, stream: &Stream) -> bool {
        let index = match self.button_index(button_id) {
            Some(i) => i,
            None => return false,
        };
        let has_window = self.buttons[index].window.is_some();

        if has_window && stream.pid > 0 {
            if self.buttons[index].pid <= 0 {
                let window = self.buttons[index].window.clone();
                if let Some(window) = window {
                    self.buttons[index].pid = crate::apps::window_pid(&window);
                }
            }
            if pid_matches(stream.pid, self.buttons[index].pid) {
                return true;
            }
            for i in 0..self.buttons.len() {
                if i == index || self.buttons[i].window.is_none() {
                    continue;
                }
                if self.buttons[i].pid <= 0 {
                    let window = self.buttons[i].window.clone();
                    if let Some(window) = window {
                        self.buttons[i].pid = crate::apps::window_pid(&window);
                    }
                }
                if pid_matches(stream.pid, self.buttons[i].pid) {
                    return false;
                }
            }
        }

        if has_window {
            let ids = self.buttons[index]
                .window
                .as_ref()
                .map(|w| crate::ffi_xfce::Window::new(w).class_ids())
                .unwrap_or_default();
            for id in &ids {
                if fuzzy_matches(id, stream.binary.as_deref()) {
                    return true;
                }
            }
        }
        if let Some(app) = self.buttons[index].app.clone() {
            let exe = crate::apps::app_executable(&app);
            let wm = app.startup_wm_class().map(|s| s.to_string());
            if fuzzy_matches(&exe, stream.binary.as_deref())
                || wm.is_some_and(|wm| fuzzy_matches(&wm, stream.binary.as_deref()))
            {
                return true;
            }
        }
        false
    }
}

/// `zd_pid_descends(s->pid, window_pid)`: is the stream owned by a process
/// below the window's process?
fn pid_matches(stream_pid: i32, window_pid: i32) -> bool {
    window_pid > 0 && crate::apps::pid_descends(stream_pid, window_pid)
}

fn fuzzy_matches(a: &str, b: Option<&str>) -> bool {
    b.is_some_and(|b| crate::apps::fuzzy_match(a, b))
}
