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

use crate::clipboard::Clipboard;
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

/// A transcription job handed to the worker (architecture section 15). The
/// worker consumes these one at a time from the bounded job channel; the
/// cancellation flag is shared between the controller and the worker
/// (architecture section 16).
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
/// mode, the recorder, the clipboard, and the bounded transcription job
/// channel; all state transitions happen here and only here.
pub struct App {
    mode: AppMode,
    next_id: u64,
    recorder: Box<dyn Recorder>,
    clipboard: Box<dyn Clipboard>,
    jobs: mpsc::SyncSender<TranscriptionJob>,
}

impl App {
    pub fn new(
        recorder: Box<dyn Recorder>,
        clipboard: Box<dyn Clipboard>,
        jobs: mpsc::SyncSender<TranscriptionJob>,
    ) -> Self {
        Self {
            mode: AppMode::Ready,
            next_id: 1,
            recorder,
            clipboard,
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

    /// Signals an orderly shutdown (architecture section 29, decision D5):
    /// requests an in-flight transcription to abort by setting its shared
    /// cancellation flag and stops an active recording, discarding its
    /// captured audio. Called by `main` after the exit key returns from the
    /// event loop. The mode itself is left untouched — dropping `App` after
    /// shutdown releases the recorder stream and the job channel, and the
    /// worker reports its stop by exiting its loop when the channels close.
    pub fn shutdown(&mut self) {
        if let AppMode::Transcribing(t) = &self.mode {
            t.cancel.store(true, Ordering::SeqCst);
        }
        // Ctrl+C during Recording must not leave the microphone stream or
        // the captured buffer alive past the exit path (plan step 3).
        // `cancel` is idempotent, so this is safe even when the recorder is
        // already idle (Ready, Transcribing, or after a prior cleanup).
        self.recorder.cancel();
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
            TranscribingPhase::Running => {
                // Successful completion (functional spec section 6): the
                // exact final text enters the persistent output area first
                // (plan step 4 — append-only history, decision D6), then the
                // same text is copied byte-for-byte, and the outcome is
                // reported. A copy failure never discards or reclassifies
                // the successful transcription (functional spec 15.4,
                // decision R4).
                let mut lines = vec!["Transcription:".to_string(), String::new(), text.clone()];
                match self.clipboard.copy_text(&text) {
                    Ok(()) => lines.push("Copied to clipboard.".to_string()),
                    Err(err) => lines.push(format!(
                        "Warning: unable to copy transcription to clipboard: {err}"
                    )),
                }
                AppOutcome {
                    view_changed: true,
                    lines,
                }
            }
            // Decision R3: a completion received after cancellation was
            // requested is discarded, even if inference finished first. No
            // text is printed or copied during the cancelling phase. The
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
    use crate::clipboard::{Clipboard, ClipboardError};
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

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ClipboardCall {
        text: String,
    }

    /// Fake clipboard that records every copy and returns a configurable
    /// result — the developer's real clipboard is never touched (architecture
    /// section 45, plan step 1).
    #[derive(Clone)]
    struct FakeClipboard {
        calls: Rc<RefCell<Vec<ClipboardCall>>>,
        result: Result<(), ClipboardError>,
    }

    impl FakeClipboard {
        fn new() -> Self {
            Self {
                calls: Rc::new(RefCell::new(Vec::new())),
                result: Ok(()),
            }
        }

        fn calls(&self) -> Vec<ClipboardCall> {
            self.calls.borrow().clone()
        }
    }

    impl Clipboard for FakeClipboard {
        fn copy_text(&mut self, text: &str) -> Result<(), ClipboardError> {
            self.calls.borrow_mut().push(ClipboardCall {
                text: text.to_string(),
            });
            self.result.clone()
        }
    }

    struct Harness {
        app: App,
        fake: FakeRecorder,
        clipboard: FakeClipboard,
        rx: mpsc::Receiver<TranscriptionJob>,
    }

    fn harness() -> Harness {
        harness_with(FakeRecorder::new())
    }

    fn harness_with(fake: FakeRecorder) -> Harness {
        harness_with_clipboard(fake, FakeClipboard::new())
    }

    fn harness_with_clipboard(fake: FakeRecorder, clipboard: FakeClipboard) -> Harness {
        let (tx, rx) = mpsc::sync_channel(1);
        Harness {
            app: App::new(Box::new(fake.clone()), Box::new(clipboard.clone()), tx),
            fake,
            clipboard,
            rx,
        }
    }

    /// Drives one full cycle to the point where a transcription outcome can
    /// be delivered: Ready → Recording → Transcribing, returning the active
    /// job id.
    fn start_transcribing(h: &mut Harness) -> TranscriptionId {
        h.app.on_command(UserCommand::ToggleRecording);
        h.app.on_command(UserCommand::ToggleRecording);
        h.rx.recv().expect("a job must be submitted").id
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
        assert!(
            h.clipboard.calls().is_empty(),
            "an ignored command never copies"
        );
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
        assert!(
            h.clipboard.calls().is_empty(),
            "a cancelled recording never copies (functional spec 14)"
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
        assert!(
            h.clipboard.calls().is_empty(),
            "an ignored command during Transcribing never copies"
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
        assert!(
            h.clipboard.calls().is_empty(),
            "commands while Cancelling never copy"
        );
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
        assert!(
            h.clipboard.calls().is_empty(),
            "a failed start never copies"
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
        assert!(h.clipboard.calls().is_empty(), "a failed stop never copies");
    }

    #[test]
    fn submission_failure_returns_to_ready() {
        let fake = FakeRecorder::new();
        let clipboard = FakeClipboard::new();
        let (tx, rx) = mpsc::sync_channel::<TranscriptionJob>(1);
        drop(rx); // worker unavailable
        let mut app = App::new(Box::new(fake.clone()), Box::new(clipboard.clone()), tx);
        app.on_command(UserCommand::ToggleRecording);

        let outcome = app.on_command(UserCommand::ToggleRecording);
        assert!(outcome.view_changed);
        assert!(matches!(app.mode(), AppMode::Ready));
        assert_eq!(
            outcome.lines,
            vec!["Error: unable to start transcription: worker unavailable"]
        );
        assert!(
            clipboard.calls().is_empty(),
            "a worker-unavailable submission never copies"
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
        assert!(
            h.clipboard.calls().is_empty(),
            "a recording failure never copies"
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
        assert!(
            h.clipboard.calls().is_empty(),
            "an out-of-state event never copies"
        );
    }

    // ---- Transcription outcomes (architecture section 17) ----

    #[test]
    fn completed_during_running_prints_text_copies_and_reports_success() {
        let mut h = harness();
        let id = start_transcribing(&mut h);

        let outcome = h.app.on_event(AppEvent::TranscriptionCompleted {
            id,
            text: "hello world".to_string(),
        });
        assert!(outcome.view_changed);
        assert!(matches!(h.app.mode(), AppMode::Ready));
        assert_eq!(
            outcome.lines,
            vec!["Transcription:", "", "hello world", "Copied to clipboard.",],
            "the final transcription is clearly labeled, then copied (plan step 3)"
        );
        assert_eq!(
            h.clipboard.calls(),
            vec![ClipboardCall {
                text: "hello world".to_string()
            }],
            "the clipboard is invoked exactly once with the printed text (functional spec 14)"
        );
    }

    #[test]
    fn copied_text_matches_printed_text_byte_for_byte() {
        let mut h = harness();
        let id = start_transcribing(&mut h);
        // Deliberately non-trivial text (whitespace, unicode, newline) so the
        // byte-identity guarantee between print and copy is actually exercised.
        let text = "Olá mundo!\n  segunda linha\tcom tab".to_string();

        let outcome = h.app.on_event(AppEvent::TranscriptionCompleted {
            id,
            text: text.clone(),
        });
        assert_eq!(outcome.lines[3], "Copied to clipboard.");
        assert_eq!(
            outcome.lines[2], text,
            "printed line must equal the copied text"
        );
        assert_eq!(
            h.clipboard.calls(),
            vec![ClipboardCall { text: text.clone() }],
            "clipboard content must match the printed transcription (functional spec 14)"
        );
    }

    #[test]
    fn clipboard_failure_keeps_transcription_and_warns() {
        let mut h = harness_with_clipboard(
            FakeRecorder::new(),
            FakeClipboard {
                result: Err(ClipboardError("no clipboard service reachable".to_string())),
                ..FakeClipboard::new()
            },
        );
        let id = start_transcribing(&mut h);

        let outcome = h.app.on_event(AppEvent::TranscriptionCompleted {
            id,
            text: "hello world".to_string(),
        });
        assert!(outcome.view_changed);
        assert!(matches!(h.app.mode(), AppMode::Ready));
        assert_eq!(
            outcome.lines,
            vec![
                "Transcription:",
                "",
                "hello world",
                "Warning: unable to copy transcription to clipboard: no clipboard service reachable",
            ],
            "a clipboard failure still prints the transcription, then warns (functional spec 15.4)"
        );
        assert_eq!(
            h.clipboard.calls(),
            vec![ClipboardCall {
                text: "hello world".to_string()
            }],
            "the copy is still attempted exactly once"
        );
    }

    #[test]
    fn completed_after_cancel_is_discarded_but_returns_to_ready() {
        let mut h = harness();
        let id = start_transcribing(&mut h);
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
        assert!(
            h.clipboard.calls().is_empty(),
            "no copy during the cancelling phase (plan step 5)"
        );
    }

    #[test]
    fn cancelled_during_cancelling_returns_to_ready() {
        let mut h = harness();
        let id = start_transcribing(&mut h);
        h.app.on_command(UserCommand::Cancel);

        let outcome = h.app.on_event(AppEvent::TranscriptionCancelled { id });
        assert!(outcome.view_changed);
        assert!(matches!(h.app.mode(), AppMode::Ready));
        assert!(outcome.lines.is_empty());
        assert!(
            h.clipboard.calls().is_empty(),
            "a cancelled transcription never copies (functional spec 14)"
        );
    }

    #[test]
    fn next_cycle_starts_after_cancel_acknowledgement() {
        // The manual check: after the worker confirms cancellation, the
        // controller returns to Ready and the next Ctrl+R starts a fresh
        // recording with a new job (plan step 4, ADR-11).
        let mut h = harness();
        let id = start_transcribing(&mut h);
        h.app.on_command(UserCommand::Cancel);
        h.app.on_event(AppEvent::TranscriptionCancelled { id });
        assert!(matches!(h.app.mode(), AppMode::Ready));

        h.fake.clear_calls();
        h.app.on_command(UserCommand::ToggleRecording);
        assert!(matches!(h.app.mode(), AppMode::Recording));
        assert_eq!(h.fake.calls(), vec![RecorderCall::Start]);

        h.app.on_command(UserCommand::ToggleRecording);
        let job = h.rx.recv().expect("the next cycle submits a job");
        assert_eq!(
            job.id,
            TranscriptionId::new(2),
            "the next job carries the next monotonic id"
        );
        assert_transcribing(h.app.mode(), TranscribingPhase::Running);
        assert!(
            h.clipboard.calls().is_empty(),
            "nothing is copied for the cancelled cycle (functional spec 14)"
        );
    }

    #[test]
    fn failed_during_running_returns_to_ready_with_error() {
        let mut h = harness();
        let id = start_transcribing(&mut h);

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
        assert!(
            h.clipboard.calls().is_empty(),
            "a failed transcription never copies (functional spec 14)"
        );
    }

    #[test]
    fn failed_during_cancelling_returns_to_ready_silently() {
        let mut h = harness();
        let id = start_transcribing(&mut h);
        h.app.on_command(UserCommand::Cancel);

        let outcome = h.app.on_event(AppEvent::TranscriptionFailed {
            id,
            message: "late failure".to_string(),
        });
        assert!(outcome.view_changed, "worker stopped: back to Ready");
        assert!(matches!(h.app.mode(), AppMode::Ready));
        assert!(outcome.lines.is_empty());
        assert!(
            h.clipboard.calls().is_empty(),
            "no copy while cancelling (plan step 5)"
        );
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
        assert!(
            h.clipboard.calls().is_empty(),
            "a stale completion never copies (functional spec 14)"
        );
    }

    #[test]
    fn stale_cancelled_and_failed_events_are_ignored() {
        // No active transcription: cancelled/failed events for any ID are
        // stale (plan step 5 — late events must never alter state).
        let mut h = harness();
        for stale in [
            AppEvent::TranscriptionCancelled {
                id: TranscriptionId::new(7),
            },
            AppEvent::TranscriptionFailed {
                id: TranscriptionId::new(7),
                message: "stale failure".to_string(),
            },
        ] {
            let outcome = h.app.on_event(stale);
            assert!(!outcome.view_changed);
            assert!(outcome.lines.is_empty());
            assert!(matches!(h.app.mode(), AppMode::Ready));
        }

        // Active transcription with id 1: cancelled/failed events for a
        // different id (2) must not disturb it.
        h.app.on_command(UserCommand::ToggleRecording);
        h.app.on_command(UserCommand::ToggleRecording);
        for stale in [
            AppEvent::TranscriptionCancelled {
                id: TranscriptionId::new(2),
            },
            AppEvent::TranscriptionFailed {
                id: TranscriptionId::new(2),
                message: "other job".to_string(),
            },
        ] {
            let outcome = h.app.on_event(stale);
            assert!(!outcome.view_changed);
            assert!(outcome.lines.is_empty());
            assert_transcribing(h.app.mode(), TranscribingPhase::Running);
        }
        assert!(
            h.clipboard.calls().is_empty(),
            "stale cancelled/failed events never copy (functional spec 14)"
        );
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
        assert!(
            h.clipboard.calls().is_empty(),
            "an unrequested cancellation never copies"
        );
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
            let calls = h.clipboard.calls();
            assert_eq!(
                calls.len(),
                expected as usize,
                "each successful cycle copies exactly once"
            );
            assert_eq!(
                calls.last().unwrap(),
                &ClipboardCall {
                    text: format!("result {expected}")
                },
                "the copy carries that cycle's result text"
            );
        }
    }

    #[test]
    fn shutdown_stops_an_active_recording() {
        let mut h = harness();
        h.app.on_command(UserCommand::ToggleRecording);
        assert!(matches!(h.app.mode(), AppMode::Recording));

        // Ctrl+C during Recording: the exit path releases the recorder
        // (stream + captured buffer) and submits nothing (plan step 3,
        // architecture section 29 step 1).
        h.app.shutdown();
        assert_eq!(
            h.fake.calls(),
            vec![RecorderCall::Start, RecorderCall::Cancel],
            "an active recording must be stopped on shutdown"
        );
        assert!(
            h.rx.try_recv().is_err(),
            "shutdown never submits a transcription job"
        );
        assert!(
            h.clipboard.calls().is_empty(),
            "shutdown never copies (functional spec 14)"
        );
    }

    #[test]
    fn shutdown_sets_the_active_cancellation_flag() {
        let mut h = harness();
        h.app.on_command(UserCommand::ToggleRecording);
        h.app.on_command(UserCommand::ToggleRecording);
        let job = h.rx.recv().unwrap();

        h.app.shutdown();
        let t = assert_transcribing(h.app.mode(), TranscribingPhase::Running);
        assert!(
            t.cancel.load(Ordering::SeqCst),
            "shutdown must request an in-flight inference to abort"
        );
        assert!(job.cancel.load(Ordering::SeqCst));
        assert!(
            matches!(h.app.mode(), AppMode::Transcribing(_)),
            "shutdown does not change the mode; the worker outcome does"
        );
    }

    // ---- Repeated mixed cycles (plan step 5) ----

    #[test]
    fn repeated_mixed_cycles_never_leak_output_or_clipboard() {
        // success -> cancel recording -> success -> cancel transcription ->
        // recoverable failure -> success (plan step 5). Every cycle starts
        // from a clean Ready, ids stay monotonic, and only successful
        // completions produce output lines and clipboard copies — cancelled
        // and failed cycles must not leak text (functional spec 14).
        let mut h = harness();

        // Cycle 1: success.
        let id1 = start_transcribing(&mut h);
        let outcome = h.app.on_event(AppEvent::TranscriptionCompleted {
            id: id1,
            text: "one".to_string(),
        });
        assert_eq!(
            outcome.lines,
            vec!["Transcription:", "", "one", "Copied to clipboard."]
        );
        assert!(matches!(h.app.mode(), AppMode::Ready));

        // Cycle 2: cancel recording — no job, no output.
        h.app.on_command(UserCommand::ToggleRecording);
        let outcome = h.app.on_command(UserCommand::Cancel);
        assert!(outcome.lines.is_empty());
        assert!(matches!(h.app.mode(), AppMode::Ready));
        assert!(
            h.rx.try_recv().is_err(),
            "a cancelled recording never submits a job"
        );

        // Cycle 3: success.
        let id3 = start_transcribing(&mut h);
        let outcome = h.app.on_event(AppEvent::TranscriptionCompleted {
            id: id3,
            text: "three".to_string(),
        });
        assert_eq!(outcome.lines[2], "three");

        // Cycle 4: cancel transcription — ready only after the worker
        // acknowledgement, and nothing is printed for the cancelled cycle.
        let id4 = start_transcribing(&mut h);
        h.app.on_command(UserCommand::Cancel);
        assert!(matches!(
            h.app.mode(),
            AppMode::Transcribing(Transcribing {
                phase: TranscribingPhase::Cancelling,
                ..
            })
        ));
        let outcome = h.app.on_event(AppEvent::TranscriptionCancelled { id: id4 });
        assert!(outcome.lines.is_empty());
        assert!(matches!(h.app.mode(), AppMode::Ready));

        // Cycle 5: recoverable transcription failure — error line, no copy.
        let id5 = start_transcribing(&mut h);
        let outcome = h.app.on_event(AppEvent::TranscriptionFailed {
            id: id5,
            message: "no speech detected".to_string(),
        });
        assert_eq!(
            outcome.lines,
            vec!["Error: transcription failed: no speech detected"]
        );
        assert!(matches!(h.app.mode(), AppMode::Ready));

        // Cycle 6: success.
        let id6 = start_transcribing(&mut h);
        assert_eq!(
            id6,
            TranscriptionId::new(5),
            "ids stay monotonic across the mixed cycles"
        );
        let outcome = h.app.on_event(AppEvent::TranscriptionCompleted {
            id: id6,
            text: "six".to_string(),
        });
        assert_eq!(outcome.lines[2], "six");
        assert!(matches!(h.app.mode(), AppMode::Ready));

        assert_eq!(
            h.clipboard.calls(),
            vec![
                ClipboardCall {
                    text: "one".to_string()
                },
                ClipboardCall {
                    text: "three".to_string()
                },
                ClipboardCall {
                    text: "six".to_string()
                },
            ],
            "only the successful cycles copy, in order — no leakage across operations"
        );
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
