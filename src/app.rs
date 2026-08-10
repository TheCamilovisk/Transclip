//! Application controller — the behavioral core (architecture sections 5-9).
//!
//! Slice 2 establishes the deterministic, hardware-free state machine driven
//! by focused-terminal commands and worker events. The controller exclusively
//! owns state transitions (architecture section 34); infrastructure returns
//! values or emits events and never mutates application state.
//!
//! Rendering is data-driven: transitions produce an [`AppOutcome`] (whether
//! the view changed plus output lines); the terminal renderer only writes
//! what the controller reports (architecture section 21).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use crate::recorder::{RecordedAudio, Recorder};
use crate::terminal::{self, Renderer, TerminalError};

/// Focused-terminal commands (functional spec section 8, architecture 7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserCommand {
    /// `Ctrl+R`: start recording when `Ready`, finish recording when
    /// `Recording`, no-op when `Transcribing`.
    ToggleRecording,
    /// `Esc`: cancel recording when `Recording`, request transcription
    /// cancellation when `Transcribing` (Running), no-op otherwise.
    Cancel,
}

/// Monotonically increasing transcription operation ID (architecture section
/// 7.2, decision R2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TranscriptionId(u64);

impl TranscriptionId {
    pub fn new(n: u64) -> Self {
        Self(n)
    }
}

/// Internal worker events (architecture section 7.2). Every transcription
/// outcome carries its operation ID so stale events are harmless.
///
/// `#[allow(dead_code)]`: the variants are the slice-4 worker contract; they
/// are constructed by the worker (slice 4) and exercised by the controller
/// tests today.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    TranscriptionCompleted {
        id: TranscriptionId,
        text: String,
    },
    TranscriptionCancelled {
        id: TranscriptionId,
    },
    TranscriptionFailed {
        id: TranscriptionId,
        message: String,
    },
    RecordingFailed(String),
}

/// A transcription job handed to the worker. Slice 4 attaches the worker that
/// consumes these from the channel; the cancellation flag is shared between
/// the controller and the worker (architecture section 16).
///
/// `#[allow(dead_code)]`: the fields are the slice-4 worker contract; they
/// are read by the worker (slice 4) and exercised by the controller tests
/// today.
#[allow(dead_code)]
#[derive(Debug)]
pub struct TranscriptionJob {
    pub id: TranscriptionId,
    pub audio: RecordedAudio,
    pub cancel: Arc<AtomicBool>,
}

/// Public application modes (functional spec section 4, architecture section
/// 6): `Ready`, `Recording`, and `Transcribing`, whose associated data holds
/// the phase, the active ID, and the cancellation flag.
#[derive(Debug, Clone)]
pub enum AppMode {
    Ready,
    Recording,
    Transcribing(Transcribing),
}

/// Transcribing-associated data (architecture section 6, decision R1).
#[derive(Debug, Clone)]
pub struct Transcribing {
    pub phase: TranscribingPhase,
    pub id: TranscriptionId,
    pub cancel: Arc<AtomicBool>,
}

/// Internal phase of `Transcribing`; `Cancelling` is not a fourth public
/// mode (functional spec section 16, decision R1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscribingPhase {
    Running,
    Cancelling,
}

impl AppMode {
    /// The status block rendered for this mode (functional spec section 11,
    /// architecture section 22). Display data owned by the controller; the
    /// terminal renderer only writes it.
    pub fn status_block(&self) -> String {
        match self {
            AppMode::Ready => "Ready to record\n\nCtrl+R  Start recording\n".to_string(),
            AppMode::Recording => "Recording...\n\nCtrl+R  Finish\nEsc     Cancel\n".to_string(),
            AppMode::Transcribing(Transcribing {
                phase: TranscribingPhase::Running,
                ..
            }) => "Transcribing...\n\nEsc     Cancel\n".to_string(),
            AppMode::Transcribing(Transcribing {
                phase: TranscribingPhase::Cancelling,
                ..
            }) => "Cancelling transcription...\n\nEsc     Cancel\n".to_string(),
        }
    }
}

/// What a transition produced, for the caller to render (architecture section
/// 21, plan step 7): whether the mode (status block) changed, and any lines
/// appended to the persistent output area (errors, results).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AppOutcome {
    pub view_changed: bool,
    pub lines: Vec<String>,
}

/// The application controller (architecture section 5.1). Owns the current
/// mode, the recorder, and the transcription job channel; all state
/// transitions happen here and only here.
pub struct App {
    mode: AppMode,
    next_id: u64,
    recorder: Box<dyn Recorder>,
    jobs: mpsc::Sender<TranscriptionJob>,
}

impl App {
    pub fn new(recorder: Box<dyn Recorder>, jobs: mpsc::Sender<TranscriptionJob>) -> Self {
        Self {
            mode: AppMode::Ready,
            next_id: 1,
            recorder,
            jobs,
        }
    }

    pub fn mode(&self) -> &AppMode {
        &self.mode
    }

    /// Applies a focused-terminal command (functional spec section 17). The
    /// full command matrix lives here in one place (plan step 3).
    pub fn on_command(&mut self, cmd: UserCommand) -> AppOutcome {
        let is_cancelling = matches!(
            &self.mode,
            AppMode::Transcribing(Transcribing {
                phase: TranscribingPhase::Cancelling,
                ..
            })
        );
        match (self.class(), cmd) {
            (ModeClass::Ready, UserCommand::ToggleRecording) => self.start_recording(),
            (ModeClass::Recording, UserCommand::ToggleRecording) => self.finish_recording(),
            (ModeClass::Recording, UserCommand::Cancel) => self.cancel_recording(),
            (ModeClass::Transcribing, UserCommand::Cancel) if !is_cancelling => {
                self.begin_cancelling()
            }
            // Ready + Cancel, Transcribing + ToggleRecording, and any command
            // while Cancelling are ignored (functional spec section 8).
            _ => AppOutcome::default(),
        }
    }

    /// Applies an internal worker event. An event is accepted only when its
    /// ID matches the active transcription and the current phase permits that
    /// outcome; everything else is discarded as stale (architecture section
    /// 17, decision R2).
    pub fn on_event(&mut self, event: AppEvent) -> AppOutcome {
        match event {
            AppEvent::TranscriptionCompleted { id, text } => {
                self.on_transcription_completed(id, text)
            }
            AppEvent::TranscriptionCancelled { id } => self.on_transcription_cancelled(id),
            AppEvent::TranscriptionFailed { id, message } => {
                self.on_transcription_failed(id, message)
            }
            AppEvent::RecordingFailed(message) => self.on_recording_failed(message),
        }
    }

    fn class(&self) -> ModeClass {
        match &self.mode {
            AppMode::Ready => ModeClass::Ready,
            AppMode::Recording => ModeClass::Recording,
            AppMode::Transcribing(_) => ModeClass::Transcribing,
        }
    }

    /// `(phase, id)` of the active transcription, if any.
    fn active_transcription(&self) -> Option<(TranscribingPhase, TranscriptionId)> {
        match &self.mode {
            AppMode::Transcribing(t) => Some((t.phase, t.id)),
            _ => None,
        }
    }

    fn start_recording(&mut self) -> AppOutcome {
        match self.recorder.start() {
            Ok(()) => {
                self.mode = AppMode::Recording;
                AppOutcome {
                    view_changed: true,
                    lines: vec![],
                }
            }
            Err(err) => AppOutcome {
                view_changed: false,
                lines: vec![format!("Error: unable to start recording: {err}")],
            },
        }
    }

    fn finish_recording(&mut self) -> AppOutcome {
        let audio = match self.recorder.stop() {
            Ok(audio) => audio,
            Err(err) => {
                self.mode = AppMode::Ready;
                return AppOutcome {
                    view_changed: true,
                    lines: vec![format!("Error: unable to stop recording: {err}")],
                };
            }
        };
        let id = TranscriptionId::new(self.next_id);
        self.next_id += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        let job = TranscriptionJob {
            id,
            audio,
            cancel: cancel.clone(),
        };
        if self.jobs.send(job).is_err() {
            // The worker is not accepting jobs (receiver dropped); there is
            // nothing to transcribe, so report and return to Ready.
            self.mode = AppMode::Ready;
            return AppOutcome {
                view_changed: true,
                lines: vec!["Error: unable to start transcription: worker unavailable".to_string()],
            };
        }
        self.mode = AppMode::Transcribing(Transcribing {
            phase: TranscribingPhase::Running,
            id,
            cancel,
        });
        AppOutcome {
            view_changed: true,
            lines: vec![],
        }
    }

    fn cancel_recording(&mut self) -> AppOutcome {
        // Discard captured audio; no transcription is ever submitted
        // (functional spec 4.2, 7.2).
        self.recorder.cancel();
        self.mode = AppMode::Ready;
        AppOutcome {
            view_changed: true,
            lines: vec![],
        }
    }

    fn begin_cancelling(&mut self) -> AppOutcome {
        if let AppMode::Transcribing(t) = &mut self.mode {
            t.cancel.store(true, Ordering::SeqCst);
            t.phase = TranscribingPhase::Cancelling;
        }
        AppOutcome {
            view_changed: true,
            lines: vec![],
        }
    }

    fn on_transcription_completed(&mut self, id: TranscriptionId, text: String) -> AppOutcome {
        let Some((phase, active)) = self.active_transcription() else {
            return AppOutcome::default();
        };
        if active != id {
            return AppOutcome::default();
        }
        self.mode = AppMode::Ready;
        match phase {
            TranscribingPhase::Running => AppOutcome {
                view_changed: true,
                // The text enters the persistent output area; clipboard copy
                // arrives in slice 5 (architecture section 18).
                lines: vec![text],
            },
            // Decision R3: a completion received after cancellation was
            // requested is discarded, even if inference finished first. The
            // worker stopping still returns us to Ready.
            TranscribingPhase::Cancelling => AppOutcome {
                view_changed: true,
                lines: vec![],
            },
        }
    }

    fn on_transcription_cancelled(&mut self, id: TranscriptionId) -> AppOutcome {
        let Some((phase, active)) = self.active_transcription() else {
            return AppOutcome::default();
        };
        if active != id {
            return AppOutcome::default();
        }
        if phase != TranscribingPhase::Cancelling {
            // The worker must not report cancellation unless requested.
            return AppOutcome::default();
        }
        self.mode = AppMode::Ready;
        AppOutcome {
            view_changed: true,
            lines: vec![],
        }
    }

    fn on_transcription_failed(&mut self, id: TranscriptionId, message: String) -> AppOutcome {
        let Some((phase, active)) = self.active_transcription() else {
            return AppOutcome::default();
        };
        if active != id {
            return AppOutcome::default();
        }
        self.mode = AppMode::Ready;
        match phase {
            TranscribingPhase::Running => AppOutcome {
                view_changed: true,
                lines: vec![format!("Error: transcription failed: {message}")],
            },
            // The worker stopped while cancelling; cancellation has
            // precedence and no error is surfaced (decision R3).
            TranscribingPhase::Cancelling => AppOutcome {
                view_changed: true,
                lines: vec![],
            },
        }
    }

    fn on_recording_failed(&mut self, message: String) -> AppOutcome {
        if !matches!(self.mode, AppMode::Recording) {
            return AppOutcome::default();
        }
        // Stop capture and discard unusable audio (functional spec 15.2).
        self.recorder.cancel();
        self.mode = AppMode::Ready;
        AppOutcome {
            view_changed: true,
            lines: vec![format!("Error: recording failed: {message}")],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModeClass {
    Ready,
    Recording,
    Transcribing,
}

/// Runs the focused-terminal command loop (architecture section 8, decision
/// D6): polls terminal input with a bounded timeout, drains worker events
/// promptly, applies transitions, and renders only when the view changed or
/// lines were produced. Never blocks on inference. Returns `Ok` on the exit
/// key (raw-mode Ctrl+C).
pub fn run(
    app: &mut App,
    worker_events: mpsc::Receiver<AppEvent>,
    renderer: &mut dyn Renderer,
) -> Result<(), TerminalError> {
    // Initial Ready render (functional spec 4.1).
    renderer
        .render(Some(&app.mode().status_block()), &[])
        .map_err(TerminalError::Write)?;

    loop {
        if terminal::poll_key(terminal::POLL_TIMEOUT)? {
            if let Some(key) = terminal::read_event()? {
                if terminal::is_exit_key(key) {
                    return Ok(());
                }
                if let Some(cmd) = terminal::map_key(key) {
                    let outcome = app.on_command(cmd);
                    render_outcome(app, outcome, renderer)?;
                }
            }
        }
        while let Ok(event) = worker_events.try_recv() {
            let outcome = app.on_event(event);
            render_outcome(app, outcome, renderer)?;
        }
    }
}

fn render_outcome(
    app: &App,
    outcome: AppOutcome,
    renderer: &mut dyn Renderer,
) -> Result<(), TerminalError> {
    if !outcome.view_changed && outcome.lines.is_empty() {
        return Ok(());
    }
    let status = if outcome.view_changed {
        Some(app.mode().status_block())
    } else {
        None
    };
    renderer
        .render(status.as_deref(), &outcome.lines)
        .map_err(TerminalError::Write)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recorder::RecorderError;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum RecorderCall {
        Start,
        Stop,
        Cancel,
    }

    /// Fake recorder that records every boundary call and returns
    /// configurable results (no microphone needed).
    #[derive(Clone)]
    struct FakeRecorder {
        calls: Rc<RefCell<Vec<RecorderCall>>>,
        start_result: Result<(), RecorderError>,
        stop_result: Result<RecordedAudio, RecorderError>,
    }

    impl FakeRecorder {
        fn new() -> Self {
            Self {
                calls: Rc::new(RefCell::new(Vec::new())),
                start_result: Ok(()),
                stop_result: Ok(RecordedAudio {
                    samples: vec![0.0; 8],
                    sample_rate: 16_000,
                }),
            }
        }

        fn calls(&self) -> Vec<RecorderCall> {
            self.calls.borrow().clone()
        }

        fn clear_calls(&self) {
            self.calls.borrow_mut().clear();
        }
    }

    impl Recorder for FakeRecorder {
        fn start(&mut self) -> Result<(), RecorderError> {
            self.calls.borrow_mut().push(RecorderCall::Start);
            self.start_result.clone()
        }

        fn stop(&mut self) -> Result<RecordedAudio, RecorderError> {
            self.calls.borrow_mut().push(RecorderCall::Stop);
            self.stop_result.clone()
        }

        fn cancel(&mut self) {
            self.calls.borrow_mut().push(RecorderCall::Cancel);
        }
    }

    struct Harness {
        app: App,
        fake: FakeRecorder,
        rx: mpsc::Receiver<TranscriptionJob>,
    }

    fn harness() -> Harness {
        harness_with(FakeRecorder::new())
    }

    fn harness_with(fake: FakeRecorder) -> Harness {
        let (tx, rx) = mpsc::channel();
        Harness {
            app: App::new(Box::new(fake.clone()), tx),
            fake,
            rx,
        }
    }

    fn transcribing(phase: TranscribingPhase, id: u64) -> AppMode {
        AppMode::Transcribing(Transcribing {
            phase,
            id: TranscriptionId::new(id),
            cancel: Arc::new(AtomicBool::new(false)),
        })
    }

    fn assert_transcribing(mode: &AppMode, expected: TranscribingPhase) -> &Transcribing {
        match mode {
            AppMode::Transcribing(t) => {
                assert_eq!(t.phase, expected, "unexpected phase");
                t
            }
            other => panic!("expected Transcribing, got {other:?}"),
        }
    }

    // ---- Command matrix (plan step 3, architecture section 44) ----

    #[test]
    fn ready_toggle_recording_starts_recorder_and_enters_recording() {
        let mut h = harness();
        let outcome = h.app.on_command(UserCommand::ToggleRecording);
        assert!(outcome.view_changed);
        assert!(outcome.lines.is_empty());
        assert!(matches!(h.app.mode(), AppMode::Recording));
        assert_eq!(h.fake.calls(), vec![RecorderCall::Start]);
    }

    #[test]
    fn ready_cancel_is_ignored() {
        let mut h = harness();
        let outcome = h.app.on_command(UserCommand::Cancel);
        assert!(!outcome.view_changed);
        assert!(outcome.lines.is_empty());
        assert!(matches!(h.app.mode(), AppMode::Ready));
        assert!(h.fake.calls().is_empty());
    }

    #[test]
    fn recording_toggle_stops_recorder_submits_job_and_enters_transcribing() {
        let mut h = harness();
        h.app.on_command(UserCommand::ToggleRecording);
        let outcome = h.app.on_command(UserCommand::ToggleRecording);
        assert!(outcome.view_changed);
        assert!(outcome.lines.is_empty());

        let t = assert_transcribing(h.app.mode(), TranscribingPhase::Running);
        assert_eq!(t.id, TranscriptionId::new(1));
        assert!(!t.cancel.load(Ordering::SeqCst));

        let job = h.rx.recv().expect("a job must be submitted");
        assert_eq!(job.id, TranscriptionId::new(1));
        assert_eq!(job.audio.samples.len(), 8);
        assert_eq!(job.audio.sample_rate, 16_000);
        assert_eq!(
            h.fake.calls(),
            vec![RecorderCall::Start, RecorderCall::Stop]
        );
    }

    #[test]
    fn recording_cancel_discards_and_submits_nothing() {
        let mut h = harness();
        h.app.on_command(UserCommand::ToggleRecording);
        let outcome = h.app.on_command(UserCommand::Cancel);
        assert!(outcome.view_changed);
        assert!(outcome.lines.is_empty());
        assert!(matches!(h.app.mode(), AppMode::Ready));
        assert_eq!(
            h.fake.calls(),
            vec![RecorderCall::Start, RecorderCall::Cancel]
        );
        assert!(
            h.rx.try_recv().is_err(),
            "a cancelled recording must not submit a job"
        );
    }

    #[test]
    fn transcribing_toggle_recording_has_no_effect() {
        let mut h = harness();
        h.app.on_command(UserCommand::ToggleRecording);
        h.app.on_command(UserCommand::ToggleRecording);
        let _first_job = h.rx.recv().expect("first job must be submitted");
        h.fake.clear_calls();

        let outcome = h.app.on_command(UserCommand::ToggleRecording);
        assert!(!outcome.view_changed);
        assert!(outcome.lines.is_empty());
        assert_transcribing(h.app.mode(), TranscribingPhase::Running);
        assert!(h.fake.calls().is_empty());
        assert!(
            h.rx.try_recv().is_err(),
            "Ctrl+R during Transcribing must not submit another job"
        );
    }

    #[test]
    fn transcribing_cancel_flips_phase_and_shared_flag() {
        let mut h = harness();
        h.app.on_command(UserCommand::ToggleRecording);
        h.app.on_command(UserCommand::ToggleRecording);
        let job = h.rx.recv().expect("a job must be submitted");

        let outcome = h.app.on_command(UserCommand::Cancel);
        assert!(outcome.view_changed);
        assert!(outcome.lines.is_empty());
        let t = assert_transcribing(h.app.mode(), TranscribingPhase::Cancelling);
        assert!(
            t.cancel.load(Ordering::SeqCst),
            "controller flag must be set"
        );
        assert!(
            job.cancel.load(Ordering::SeqCst),
            "the worker's copy of the flag must be set"
        );
    }

    #[test]
    fn cancelling_accepts_no_commands() {
        let mut h = harness();
        h.app.on_command(UserCommand::ToggleRecording);
        h.app.on_command(UserCommand::ToggleRecording);
        h.app.on_command(UserCommand::Cancel);
        h.fake.clear_calls();

        let outcome = h.app.on_command(UserCommand::ToggleRecording);
        assert!(!outcome.view_changed);
        assert!(outcome.lines.is_empty());
        assert_transcribing(h.app.mode(), TranscribingPhase::Cancelling);

        let outcome = h.app.on_command(UserCommand::Cancel);
        assert!(!outcome.view_changed);
        assert!(outcome.lines.is_empty());
        assert_transcribing(h.app.mode(), TranscribingPhase::Cancelling);
        assert!(h.fake.calls().is_empty());
    }

    // ---- Error paths (functional spec section 15) ----

    #[test]
    fn ready_start_failure_stays_ready_and_surfaces_error() {
        let mut h = harness_with(FakeRecorder {
            start_result: Err(RecorderError("unable to access microphone".to_string())),
            ..FakeRecorder::new()
        });

        let outcome = h.app.on_command(UserCommand::ToggleRecording);
        assert!(!outcome.view_changed);
        assert!(matches!(h.app.mode(), AppMode::Ready));
        assert_eq!(
            outcome.lines,
            vec!["Error: unable to start recording: unable to access microphone"]
        );
    }

    #[test]
    fn recording_stop_failure_returns_to_ready_and_surfaces_error() {
        let mut h = harness_with(FakeRecorder {
            stop_result: Err(RecorderError("stream died".to_string())),
            ..FakeRecorder::new()
        });
        h.app.on_command(UserCommand::ToggleRecording);

        let outcome = h.app.on_command(UserCommand::ToggleRecording);
        assert!(outcome.view_changed);
        assert!(matches!(h.app.mode(), AppMode::Ready));
        assert_eq!(
            outcome.lines,
            vec!["Error: unable to stop recording: stream died"]
        );
        assert!(h.rx.try_recv().is_err(), "no job on a failed stop");
    }

    #[test]
    fn submission_failure_returns_to_ready() {
        let fake = FakeRecorder::new();
        let (tx, rx) = mpsc::channel::<TranscriptionJob>();
        drop(rx); // worker unavailable
        let mut app = App::new(Box::new(fake.clone()), tx);
        app.on_command(UserCommand::ToggleRecording);

        let outcome = app.on_command(UserCommand::ToggleRecording);
        assert!(outcome.view_changed);
        assert!(matches!(app.mode(), AppMode::Ready));
        assert_eq!(
            outcome.lines,
            vec!["Error: unable to start transcription: worker unavailable"]
        );
    }

    #[test]
    fn recording_failed_event_discards_and_returns_to_ready() {
        let mut h = harness();
        h.app.on_command(UserCommand::ToggleRecording);

        let outcome = h
            .app
            .on_event(AppEvent::RecordingFailed("device disconnected".to_string()));
        assert!(outcome.view_changed);
        assert!(matches!(h.app.mode(), AppMode::Ready));
        assert_eq!(
            outcome.lines,
            vec!["Error: recording failed: device disconnected"]
        );
        assert_eq!(
            h.fake.calls(),
            vec![RecorderCall::Start, RecorderCall::Cancel],
            "unusable audio must be discarded"
        );
    }

    #[test]
    fn recording_failed_outside_recording_is_ignored() {
        let mut h = harness();
        let outcome = h
            .app
            .on_event(AppEvent::RecordingFailed("noise".to_string()));
        assert!(!outcome.view_changed);
        assert!(outcome.lines.is_empty());
        assert!(matches!(h.app.mode(), AppMode::Ready));
        assert!(h.fake.calls().is_empty());
    }

    // ---- Transcription outcomes (architecture section 17) ----

    #[test]
    fn completed_during_running_prints_text_and_returns_to_ready() {
        let mut h = harness();
        h.app.on_command(UserCommand::ToggleRecording);
        h.app.on_command(UserCommand::ToggleRecording);
        let id = h.rx.recv().unwrap().id;

        let outcome = h.app.on_event(AppEvent::TranscriptionCompleted {
            id,
            text: "hello world".to_string(),
        });
        assert!(outcome.view_changed);
        assert!(matches!(h.app.mode(), AppMode::Ready));
        assert_eq!(outcome.lines, vec!["hello world"]);
    }

    #[test]
    fn completed_after_cancel_is_discarded_but_returns_to_ready() {
        let mut h = harness();
        h.app.on_command(UserCommand::ToggleRecording);
        h.app.on_command(UserCommand::ToggleRecording);
        let id = h.rx.recv().unwrap().id;
        h.app.on_command(UserCommand::Cancel);

        let outcome = h.app.on_event(AppEvent::TranscriptionCompleted {
            id,
            text: "late result".to_string(),
        });
        assert!(outcome.view_changed, "the worker stopping returns to Ready");
        assert!(matches!(h.app.mode(), AppMode::Ready));
        assert!(
            outcome.lines.is_empty(),
            "a completion after cancellation must be discarded (R3)"
        );
    }

    #[test]
    fn cancelled_during_cancelling_returns_to_ready() {
        let mut h = harness();
        h.app.on_command(UserCommand::ToggleRecording);
        h.app.on_command(UserCommand::ToggleRecording);
        let id = h.rx.recv().unwrap().id;
        h.app.on_command(UserCommand::Cancel);

        let outcome = h.app.on_event(AppEvent::TranscriptionCancelled { id });
        assert!(outcome.view_changed);
        assert!(matches!(h.app.mode(), AppMode::Ready));
        assert!(outcome.lines.is_empty());
    }

    #[test]
    fn failed_during_running_returns_to_ready_with_error() {
        let mut h = harness();
        h.app.on_command(UserCommand::ToggleRecording);
        h.app.on_command(UserCommand::ToggleRecording);
        let id = h.rx.recv().unwrap().id;

        let outcome = h.app.on_event(AppEvent::TranscriptionFailed {
            id,
            message: "no speech detected".to_string(),
        });
        assert!(outcome.view_changed);
        assert!(matches!(h.app.mode(), AppMode::Ready));
        assert_eq!(
            outcome.lines,
            vec!["Error: transcription failed: no speech detected"]
        );
    }

    #[test]
    fn failed_during_cancelling_returns_to_ready_silently() {
        let mut h = harness();
        h.app.on_command(UserCommand::ToggleRecording);
        h.app.on_command(UserCommand::ToggleRecording);
        let id = h.rx.recv().unwrap().id;
        h.app.on_command(UserCommand::Cancel);

        let outcome = h.app.on_event(AppEvent::TranscriptionFailed {
            id,
            message: "late failure".to_string(),
        });
        assert!(outcome.view_changed, "worker stopped: back to Ready");
        assert!(matches!(h.app.mode(), AppMode::Ready));
        assert!(outcome.lines.is_empty());
    }

    #[test]
    fn stale_events_are_discarded() {
        // No active transcription: any transcription event is stale.
        let mut h = harness();
        let outcome = h.app.on_event(AppEvent::TranscriptionCompleted {
            id: TranscriptionId::new(7),
            text: "stale".to_string(),
        });
        assert!(!outcome.view_changed);
        assert!(outcome.lines.is_empty());
        assert!(matches!(h.app.mode(), AppMode::Ready));

        // Active transcription with id 1: an event for id 2 is stale.
        h.app.on_command(UserCommand::ToggleRecording);
        h.app.on_command(UserCommand::ToggleRecording);
        let outcome = h.app.on_event(AppEvent::TranscriptionCompleted {
            id: TranscriptionId::new(2),
            text: "other job".to_string(),
        });
        assert!(!outcome.view_changed);
        assert!(outcome.lines.is_empty());
        assert_transcribing(h.app.mode(), TranscribingPhase::Running);
    }

    #[test]
    fn cancelled_without_request_is_ignored() {
        let mut h = harness();
        h.app.on_command(UserCommand::ToggleRecording);
        h.app.on_command(UserCommand::ToggleRecording);
        let id = h.rx.recv().unwrap().id;

        let outcome = h.app.on_event(AppEvent::TranscriptionCancelled { id });
        assert!(!outcome.view_changed);
        assert!(outcome.lines.is_empty());
        assert_transcribing(h.app.mode(), TranscribingPhase::Running);
    }

    #[test]
    fn ids_are_monotonic_across_cycles() {
        let mut h = harness();
        for expected in 1..=2u64 {
            h.app.on_command(UserCommand::ToggleRecording);
            h.app.on_command(UserCommand::ToggleRecording);
            let job = h.rx.recv().unwrap();
            assert_eq!(job.id, TranscriptionId::new(expected));
            let outcome = h.app.on_event(AppEvent::TranscriptionCompleted {
                id: job.id,
                text: format!("result {expected}"),
            });
            assert!(outcome.view_changed);
            assert!(matches!(h.app.mode(), AppMode::Ready));
        }
    }

    // ---- Display data (functional spec section 11) ----

    #[test]
    fn status_blocks_are_distinct_per_mode() {
        assert_eq!(
            AppMode::Ready.status_block(),
            "Ready to record\n\nCtrl+R  Start recording\n"
        );
        assert_eq!(
            AppMode::Recording.status_block(),
            "Recording...\n\nCtrl+R  Finish\nEsc     Cancel\n"
        );
        assert_eq!(
            transcribing(TranscribingPhase::Running, 1).status_block(),
            "Transcribing...\n\nEsc     Cancel\n"
        );
        assert_eq!(
            transcribing(TranscribingPhase::Cancelling, 1).status_block(),
            "Cancelling transcription...\n\nEsc     Cancel\n"
        );
    }
}
