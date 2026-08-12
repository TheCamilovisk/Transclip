# Functional Specification — Terminal Voice Transcriber

## 1. Purpose

The application is a terminal-based voice transcription utility that allows the user to:

- start and stop voice recording using keyboard shortcuts;
- cancel an active recording;
- transcribe the recorded audio into text;
- cancel an active transcription;
- print the resulting transcription in the terminal;
- automatically copy the transcribed text to the system clipboard.

The application is intended to remain active in the terminal and wait for repeated recording commands.

The initial release supports Linux desktop environments only. Voice recognition executes locally after the required Whisper model has been provisioned.

---

## 2. Scope

### 2.1 In Scope

The application shall support:

- terminal-based interaction;
- microphone audio recording;
- local keyboard commands;
- voice-to-text transcription;
- automatic source-language detection for each transcription;
- transcription cancellation;
- recording cancellation;
- terminal status feedback;
- clipboard integration;
- repeated recording/transcription cycles within the same application session.

The application shall download and verify its required Whisper model before its first usable session when the model is not already present locally.

### 2.2 Out of Scope

The initial version does not include:

- graphical user interface;
- global operating-system keyboard shortcuts;
- background interaction while the terminal is unfocused;
- audio file persistence;
- transcription history persistence;
- user accounts;
- cloud synchronization;
- transcription editing;
- streaming partial transcription;
- multiple simultaneous recordings;
- configurable keyboard shortcuts;
- automatic voice activity detection.

---

# 3. User Interaction Model

The application operates through keyboard commands while its terminal window is focused.

The application has three primary modes:

1. **Ready to Record**
2. **Recording**
3. **Transcribing**

Only commands valid for the current mode shall trigger actions.

---

# 4. Application Modes

## 4.1 Ready to Record

The application is idle and waiting for the user to begin recording.

The terminal shall indicate that the application is ready to record.

Example:

```text
Ready to record

Ctrl+R  Start recording
```

### Available Commands

- `Ctrl+R` — start recording.

### Result

When `Ctrl+R` is pressed:

1. microphone recording starts;
2. the application changes to **Recording** mode;
3. the terminal display is updated to indicate that recording is active.

---

## 4.2 Recording

The application is actively capturing audio from the configured microphone.

The terminal shall visually indicate that recording is in progress.

Example:

```text
Recording...

Ctrl+R  Finish recording
Esc     Cancel
```

### Available Commands

#### `Ctrl+R`

Stops the recording and begins transcription.

The application shall:

1. stop capturing microphone audio;
2. preserve the captured audio for transcription;
3. change to **Transcribing** mode;
4. start transcription;
5. update the terminal status.

#### `Esc`

Cancels the active recording.

The application shall:

1. stop microphone capture;
2. discard all audio captured during the current recording;
3. not start transcription;
4. return to **Ready to Record** mode.

No text shall be copied to the clipboard after a cancelled recording.

---

# 5. Transcribing Mode

The application enters this mode after the user finishes a recording.

The terminal shall visually indicate that transcription is in progress.

Example:

```text
Transcribing...

Esc     Cancel
```

Transcription must execute without preventing the application from processing keyboard input.

After cancellation is requested, the terminal shall show that cancellation is in progress until the transcription worker confirms it has stopped. The application remains in **Transcribing** mode during this internal cancellation phase and shall not accept a new recording command. This prevents concurrent use of the local transcription model.

### Available Commands

#### `Esc`

Cancels the active transcription.

The application shall:

1. request cancellation of the transcription operation;
2. discard any incomplete transcription result;
3. display that cancellation is in progress;
4. wait for the transcription worker to stop and release its resources;
5. return to **Ready to Record** mode.

No incomplete text shall be printed or copied to the clipboard.

---

# 6. Successful Transcription Flow

When transcription completes successfully, the application shall:

1. automatically detect the recording's spoken language and retrieve its complete transcription without translating it to English;
2. normalize the complete transcription text;
3. print the text in the terminal;
4. copy the complete text to the system clipboard;
5. indicate that the text was copied successfully;
6. return to **Ready to Record** mode.

Example fixed interface after a successful transcription:

```text
Ready to record

This is the text that was recorded.

Copied to clipboard.

Ctrl+R  Start recording
```

The application shall retain and display only the most recent successful transcription for the remainder of the session or until a later successful transcription replaces it. A new successful transcription shall replace the previous one in the same transcription area; it shall not be appended below the previous transcription. Cancelled or failed recordings shall not replace the displayed successful transcription.

The successful transcription is canonical plain text. Decoder or timestamp segment boundaries shall not become user-visible line breaks. The application shall trim segment-boundary whitespace, discard empty segments, and join the remaining text with a single space. This canonical text is the text printed as the result and the exact text copied to the clipboard.

---

# 7. Main User Flows

## 7.1 Record and Transcribe

### Preconditions

- the application is running;
- the terminal is focused;
- the application is in **Ready to Record** mode;
- a usable microphone is available.

### Flow

1. User presses `Ctrl+R`.
2. Application starts recording.
3. Application enters **Recording** mode.
4. User speaks.
5. User presses `Ctrl+R`.
6. Application stops recording.
7. Application enters **Transcribing** mode.
8. Application transcribes the recorded audio.
9. Application prints the transcription.
10. Application copies the transcription to the clipboard.
11. Application returns to **Ready to Record** mode.

### Postconditions

- the transcription is visible in the terminal;
- the transcription is available in the system clipboard;
- the application is ready for another recording.

---

## 7.2 Cancel Recording

### Preconditions

- the application is in **Recording** mode.

### Flow

1. User presses `Esc`.
2. Application stops recording.
3. Application discards the captured audio.
4. Application returns to **Ready to Record** mode.

### Postconditions

- no transcription is started;
- no text is copied to the clipboard;
- the application is ready for another recording.

---

## 7.3 Cancel Transcription

### Preconditions

- the application is in **Transcribing** mode.

### Flow

1. User presses `Esc`.
2. Application requests transcription cancellation.
3. Application stops or abandons the transcription operation.
4. Application discards incomplete transcription output.
5. Application displays cancellation progress until the worker stops.
6. Application returns to **Ready to Record** mode.

### Postconditions

- no incomplete transcription is printed as a successful result;
- no incomplete transcription is copied to the clipboard;
- the application is ready for another recording.

---

# 8. Keyboard Behavior

## 8.1 Ctrl+R

`Ctrl+R` has state-dependent behavior.

| Current Mode    | Behavior                                 |
| --------------- | ---------------------------------------- |
| Ready to Record | Start recording                          |
| Recording       | Finish recording and start transcription |
| Transcribing    | No action                                |

The application shall prevent unsupported state transitions caused by `Ctrl+R`.

---

## 8.2 Esc

`Esc` has state-dependent behavior.

| Current Mode    | Behavior             |
| --------------- | -------------------- |
| Ready to Record | No action            |
| Recording       | Cancel recording     |
| Transcribing    | Cancel transcription |

---

# 9. Terminal Focus Requirement

Keyboard commands shall only be handled when they are delivered to the terminal running the application.

The application shall not register `Ctrl+R`, `Esc`, or any other command as a global operating-system shortcut.

As a consequence:

- when the terminal is focused, supported keyboard commands control the application;
- when another application is focused, those keyboard commands shall not control the voice transcription application.

---

# 10. Responsiveness Requirements

The application shall remain responsive while:

- waiting for recording;
- recording audio;
- transcribing audio.

In particular:

### During Recording

The application must continue accepting:

- `Ctrl+R`;
- `Esc`.

Audio capture must not block keyboard event processing.

### During Transcription

The application must continue accepting:

- `Esc`.

Transcription processing must not block keyboard event processing.

---

# 11. Visual Feedback

The current application mode shall always be understandable from the terminal display.

At minimum, the application shall display a distinct indication for:

- Ready to Record;
- Recording;
- Transcribing.

The terminal shall also display the keyboard commands available in the current mode.

Every displayed logical line, including status text, blank lines, and transcription output, shall begin at the terminal's left margin. Enabling per-key terminal input mode shall not cause a later line to retain the preceding line's cursor column. Terminal line-ending serialization is presentation-only and shall not change the canonical transcription text.

The terminal is a fixed interactive interface rather than append-only command output. Rendering a state change, a notice, or a new successful transcription shall redraw the existing interface in place without duplicating status text, keyboard commands, errors, warnings, or previous transcription text in terminal history. The interface shall contain the current status, the most recent successful transcription when one exists, the available commands, and at most one transient notice. A new notice replaces the prior notice; a new successful transcription replaces the prior successful transcription. A transient notice does not remove the most recent successful transcription.

Native Whisper and GGML diagnostic output, including model-loading information, shall not be displayed in the terminal. The fixed application interface is the first normal terminal output after successful startup.

Example:

```text
Ready to record
Ctrl+R  Start recording
```

```text
Recording...
Ctrl+R  Finish
Esc     Cancel
```

```text
Transcribing...
Esc     Cancel
```

Exact layout, colors, symbols, and animations are implementation details, provided the status, latest transcription, current commands, and any transient notice remain understandable.

---

# 12. Audio Capture Requirements

The application shall:

- record audio from an available microphone;
- capture only one recording at a time;
- stop capturing audio when the user finishes or cancels the recording;
- use the captured audio as input to transcription only when recording is successfully finished.

Cancelled recordings shall not be transcribed.

Audio persistence beyond the active recording/transcription cycle is not required.

---

# 13. Transcription Requirements

The transcription component shall receive audio captured during the current recording.

It shall produce plain text representing the recognized speech.

When the recognition engine returns text in multiple decoder or timestamp segments, those boundaries are internal implementation details. The final plain text shall be whitespace-normalized by trimming segment-boundary whitespace, discarding empty segments, and separating the remaining text with one space. The application shall not add line breaks or wrapping at decoder segment boundaries.

The first version requires only a final transcription.

The following are not required:

- partial transcription output;
- word-level timestamps;
- speaker identification;
- punctuation configuration;
- transcription history.

---

# 14. Clipboard Requirements

After a successful transcription:

- the complete transcription shall be copied to the system clipboard;
- clipboard content shall exactly match the canonical transcription text printed as the successful transcription result; terminal-only line-ending serialization shall not affect clipboard content.

Clipboard operations shall not occur when:

- recording is cancelled;
- transcription is cancelled;
- transcription fails.

---

# 15. Error Handling

The application shall handle recoverable errors without unexpectedly terminating whenever practical.

## 15.1 Microphone Error

If recording cannot start, the application shall:

1. display an error message;
2. remain or return to **Ready to Record** mode.

Example:

```text
Error: unable to access microphone.

Ready to record
```

---

## 15.2 Recording Error

If audio capture fails during recording, the application shall:

1. stop the active recording;
2. discard unusable audio;
3. display an error;
4. return to **Ready to Record** mode.

---

## 15.3 Transcription Error

If transcription fails, the application shall:

1. display an error message;
2. not copy text to the clipboard;
3. return to **Ready to Record** mode.

---

## 15.4 Clipboard Error

If transcription succeeds but copying to the clipboard fails:

1. the transcription shall still be printed;
2. the application shall display a clipboard error;
3. the application shall return to **Ready to Record** mode.

Example:

```text
Transcription:

This is the transcribed text.

Warning: unable to copy transcription to clipboard.

Ready to record
```

A clipboard failure shall not invalidate a successful transcription.

---

## 15.5 Model Provisioning or Initialization Error

If the required local Whisper model is missing, the application shall download its pinned model artifact before entering **Ready to Record**. The download shall be integrity-verified. If downloading, verification, or loading the model fails, the application shall display a clear startup error and exit without enabling recording.

---

# 16. Application State Model

The normal state transitions are:

```text
Ready
  |
  | Ctrl+R
  v
Recording
  |
  | Ctrl+R
  v
Transcribing
  |
  | success
  v
Ready
```

Recording cancellation:

```text
Recording
  |
  | Esc
  v
Ready
```

Transcription cancellation:

```text
Transcribing
  |
  | Esc
  v
Cancelling transcription
  |
  | worker stopped
  v
Ready
```

`Cancelling transcription` is an internal phase of **Transcribing**, not a fourth public application mode.

Error recovery:

```text
Recording
  |
  | error
  v
Ready
```

```text
Transcribing
  |
  | error
  v
Ready
```

---

# 17. Functional State Table

| Current State | Event                  | Action                                 | Next State   |
| ------------- | ---------------------- | -------------------------------------- | ------------ |
| Ready         | `Ctrl+R`               | Start recording                        | Recording    |
| Ready         | `Esc`                  | None                                   | Ready        |
| Recording     | `Ctrl+R`               | Stop recording and start transcription | Transcribing |
| Recording     | `Esc`                  | Cancel and discard recording           | Ready        |
| Recording     | Recording failure      | Display error                          | Ready        |
| Transcribing  | `Esc`                  | Request cancellation; await worker stop | Transcribing |
| Transcribing (cancelling) | Worker stopped | Discard result and display ready | Ready |
| Transcribing  | Transcription succeeds | Print and copy text                    | Ready        |
| Transcribing  | Transcription fails    | Display error                          | Ready        |

---

# 18. Session Behavior

The application is designed to remain running across multiple transcription cycles.

For example:

```text
Start application
      ↓
Record
      ↓
Transcribe
      ↓
Ready
      ↓
Record
      ↓
Cancel
      ↓
Ready
      ↓
Record
      ↓
Transcribe
      ↓
Ready
```

The user shall not need to restart the application between recordings.

---

# 19. Functional Acceptance Criteria

The initial version shall be considered functionally complete when all of the following behaviors work:

1. Starting the application provisions and verifies the local Whisper model when necessary, then displays **Ready to Record**.

2. Pressing `Ctrl+R` while ready starts microphone recording.

3. Recording mode clearly indicates that recording is active.

4. Pressing `Ctrl+R` while recording stops recording and starts transcription.

5. Pressing `Esc` while recording cancels the recording and returns to ready mode.

6. Transcribing mode clearly indicates that transcription is active.

7. The terminal remains responsive while transcription is running.

8. Pressing `Esc` while transcribing displays cancellation progress, discards the result, and returns to ready mode only after the worker stops.

9. Successful transcription is printed in the terminal.

10. Successful transcription is copied to the system clipboard.

11. The application automatically returns to ready mode after successful transcription.

12. The user can execute multiple recording/transcription cycles without restarting the application.

13. Keyboard commands only affect the application when the terminal receives those keyboard events.

14. Cancelled recordings do not trigger transcription.

15. Cancelled transcriptions do not produce successful output.

16. Recoverable recording or transcription failures return the application to ready mode.

17. Clipboard failure does not discard an otherwise successful transcription.

18. Multisegment transcription is displayed as normalized plain text without decoder-segment line breaks, copied unchanged to the clipboard, and every terminal output line remains left-aligned while per-key terminal input mode is active.

19. A Portuguese recording is automatically detected and transcribed in Portuguese rather than translated to English; a later recording in another supported language is detected independently.

20. The terminal UI remains fixed throughout the session: status, current commands, one latest successful transcription, and at most one transient notice are redrawn in place. A later successful transcription replaces the displayed text without growing terminal output or duplicating the interface.

21. The status line uses the state-specific emoji and color: light-green `🟢` when ready, light-red `🔴` when recording, and neutral `⚙️` when transcribing or cancelling. Transcription text, notices, command hints, and Whisper/GGML output remain neutral.

---

# 20. Initial Functional Boundary

The first version should remain intentionally narrow:

```text
Keyboard
   ↓
Record audio
   ↓
Transcribe
   ↓
Print text
   ↓
Copy to clipboard
   ↓
Ready again
```

Any functionality beyond this flow should be treated as a future enhancement rather than part of the initial implementation.
