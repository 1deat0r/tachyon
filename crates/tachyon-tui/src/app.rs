//! Public entrypoint for `tachyon attach` (plan item 2): three
//! independent paths — an input task, a gateway-event reader task, and a
//! render loop that draws only on state change or a bounded tick (≤ 30 FPS).

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tachyon_protocol::Command;
use tachyon_types::TaskId;
use tokio::sync::mpsc;

use crate::input::{Key, Outbound, handle_key};
use crate::poll::PollGate;
use crate::reader::{AttachConfig, ClientEvent, Subscription};
use crate::render::{RENDER_TICK, draw, should_draw};
use crate::state::AppState;

/// `tachyon attach [TASK]` options: attach straight to a task, or to
/// nothing (the picker starts on the task list).
#[derive(Debug, Default, Clone)]
pub struct AttachOptions {
    /// Task id to attach to up front; `None` starts the task picker.
    pub task_id: Option<String>,
}

/// Errors from [`run`].
#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    /// Socket/terminal I/O (connecting happens first, so a dead endpoint
    /// is a clean error before any terminal state changes).
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
    /// `--task` was not a parseable task id.
    #[error("invalid task id {0:?}: {1}")]
    InvalidTaskId(String, String),
    /// Protocol framing/validation failure.
    #[error("protocol error: {0}")]
    Protocol(#[from] tachyon_protocol::ProtocolError),
    /// Terminal setup/teardown or a draw failed.
    #[error("terminal error: {0}")]
    Terminal(String),
}

/// Runs the TUI against the gateway at `address` (the same endpoint type
/// [`tachyon_gateway::transport::connect`] takes: a socket path) until
/// the user detaches (Ctrl+Q / Ctrl+C) or the reader gives up.
pub async fn run(address: &Path, options: AttachOptions) -> Result<(), TuiError> {
    // (1) Connect FIRST: a dead endpoint must fail before the terminal
    // is ever mutated (raw mode, alternate screen).
    let commands = crate::command::CommandClient::connect(address).await?;

    // (2) Parse the attach target (typed error, still pre-terminal).
    let task = match options.task_id {
        Some(raw) => Some(
            raw.parse::<TaskId>()
                .map_err(|error| TuiError::InvalidTaskId(raw, error.to_string()))?,
        ),
        None => None,
    };

    // (3) Spawn the reader path (two connections per attach, D2 — the
    // reader owns its own bounded reconnect backoff).
    let mut subscription = Subscription::attach(address, task, -1, AttachConfig::production());
    let mut state = AppState::new(task);

    // (4) Terminal + three-path loop.
    let mut terminal = init_terminal()?;
    let outcome = run_loop(&mut terminal, commands, &mut subscription, &mut state).await;
    restore_terminal(&mut terminal);
    outcome
}

/// D3 poll: fire `ListTasks` and forward the reply to the state owner.
fn spawn_list_tasks(
    client: crate::command::CommandClient,
    responses: mpsc::UnboundedSender<tachyon_protocol::ResponseEnvelope>,
) {
    tokio::spawn(async move {
        if let Ok(response) = client.call(Command::ListTasks { session_id: None }).await {
            let _ = responses.send(tachyon_protocol::ResponseEnvelope {
                protocol_version: tachyon_protocol::PROTOCOL_VERSION,
                request_id: tachyon_types::EventId::generate(),
                result: tachyon_protocol::CommandResult::Ok { payload: response },
            });
        }
    });
}

/// The three-path loop: input task, reader pump, bounded render tick.
async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    commands: crate::command::CommandClient,
    subscription: &mut Subscription,
    state: &mut AppState,
) -> Result<(), TuiError> {
    let (key_tx, mut key_rx) = mpsc::channel::<Key>(64);
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Outbound>();
    // Async callers (poll spawn tasks) hand responses back here so the
    // state owner stays single-threaded.
    let (resp_tx, mut resp_rx) = mpsc::unbounded_channel();

    // Input path: a blocking task pumping crossterm events into `Key`s.
    let input_running = Arc::new(AtomicBool::new(true));
    let input_flag = Arc::clone(&input_running);
    let input_task = tokio::task::spawn_blocking(move || input_task(&input_flag, &key_tx));

    let mut ticker = tokio::time::interval(RENDER_TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut poll_gate = PollGate::new(std::time::Duration::from_secs(1));
    let mut dirty = true;
    let mut last_draw = Instant::now()
        .checked_sub(RENDER_TICK)
        .unwrap_or_else(Instant::now);
    let mut reader_gone = false;

    'outer: loop {
        tokio::select! {
            // ---- input path --------------------------------------------
            key = key_rx.recv() => {
                let Some(key) = key else { break 'outer; };
                handle_key(state, key, &out_tx);
                dirty = true;
            }
            // ---- gateway-event reader path ----------------------------
            event = subscription.recv(), if !reader_gone => {
                match event {
                    Some(ClientEvent::Envelope(envelope)) => {
                        state.apply_envelope(envelope);
                        dirty = true;
                    }
                    Some(ClientEvent::Ack {
                        last_seq,
                        provider_label,
                        ..
                    }) => {
                        // Replay drained; last_seq is informational (the
                        // status bar shows the applied cursor). The ack may
                        // declare the provider label (plan G5).
                        state.note_provider_label(provider_label);
                        let _ = last_seq;
                        dirty = true;
                    }
                    Some(ClientEvent::Reconnecting { attempt }) => {
                        state.notice = Some(format!("gateway unreachable — reconnect attempt {attempt}"));
                        dirty = true;
                    }
                    Some(ClientEvent::GaveUp { reason }) => {
                        state.notice = Some(format!("subscription ended: {reason}"));
                        dirty = true;
                        reader_gone = true; // observe terminal once; keep drawing
                    }
                    None => {
                        state.notice = Some("subscription reader stopped".to_owned());
                        dirty = true;
                        reader_gone = true;
                    }
                }
            }
            // ---- outbound commands from the input path -----------------
            outbound = out_rx.recv() => {
                match outbound {
                    Some(Outbound::Command(command)) => {
                        // Fire-and-forget: typed errors surface as a notice.
                        let client = commands.clone();
                        let responses = resp_tx.clone();
                        tokio::spawn(async move {
                            if let Ok(response) = client.call_raw(command).await {
                                let _ = responses.send(response);
                            }
                        });
                    }
                    Some(Outbound::Subscribe { task_id, after_seq }) => {
                        subscription.switch(task_id, after_seq);
                        dirty = true;
                    }
                    Some(Outbound::Quit) | None => break 'outer, // detach: never a cancel; input half closed
                }
            }
            // ---- responses arriving from spawned calls -----------------
            response = resp_rx.recv() => {
                if let Some(response) = response {
                    state.apply_response(response);
                    dirty = true;
                }
            }
            // ---- bounded tick: render pacing + D3 poll gate ------------
            _ = ticker.tick() => {
                let visible = state.pane == crate::state::Pane::Tasks;
                if poll_gate.should_poll(visible, Instant::now()) {
                    spawn_list_tasks(commands.clone(), resp_tx.clone());
                }
                if should_draw(dirty, last_draw, Instant::now()) {
                    terminal
                        .draw(|frame| draw(frame, state))
                        .map_err(|error| TuiError::Terminal(error.to_string()))?;
                    dirty = false;
                    last_draw = Instant::now();
                }
            }
        }
    }

    // Detach: stop the input task; the subscription drops with `subscription`
    // when `run` returns — no CancelTask is ever produced here (tested).
    input_running.store(false, Ordering::SeqCst);
    let _ = input_task.await;
    Ok(())
}

/// crossterm key pump for the production input path (no TTY in tests —
/// the mapping itself is covered by the input-mapping unit tests over
/// the crate-local [`Key`] type).
fn input_task(running: &Arc<AtomicBool>, key_tx: &mpsc::Sender<Key>) {
    while running.load(Ordering::SeqCst) {
        match crossterm::event::poll(std::time::Duration::from_millis(50)) {
            Ok(true) => {
                let Ok(CrosstermEvent::Key(event)) = crossterm::event::read() else {
                    continue; // resize/paste/mouse — ignored (no mouse, M11)
                };
                if event.kind == KeyEventKind::Release {
                    continue;
                }
                let Some(key) = map_key(event) else {
                    continue;
                };
                if key_tx.blocking_send(key).is_err() {
                    break; // loop is shutting down
                }
            }
            Ok(false) => {}
            Err(_) => break, // terminal gone
        }
    }
}

/// Maps one crossterm key event onto the crate-local [`Key`] vocabulary.
/// Returns `None` for keys the TUI does not bind (ignored, not fatal).
fn map_key(event: KeyEvent) -> Option<Key> {
    let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
    match event.code {
        KeyCode::Char(character) if ctrl => Some(Key::Ctrl(character)),
        KeyCode::Char(character) => Some(Key::Char(character)),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Esc => Some(Key::Esc),
        KeyCode::Tab => Some(Key::Tab),
        KeyCode::BackTab => Some(Key::BackTab),
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        _ => None,
    }
}

/// Enters raw mode + the alternate screen and builds the terminal.
fn init_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>, TuiError> {
    enable_raw_mode().map_err(|error| TuiError::Terminal(error.to_string()))?;
    execute!(io::stdout(), EnterAlternateScreen)
        .map_err(|error| TuiError::Terminal(error.to_string()))?;
    Terminal::new(CrosstermBackend::new(io::stdout())).map_err(|error| {
        // Best effort: never leave the caller's shell in raw mode.
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        TuiError::Terminal(error.to_string())
    })
}

/// Restores the terminal on every exit path (success, error, detach).
fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();
}
