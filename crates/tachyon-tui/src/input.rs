//! Input path: key events → state edits + outbound commands.
//!
//! The mapping is pure over a crate-local [`Key`] so tests inject synthetic
//! keys with no TTY; crossterm translation happens once at the boundary in
//! the production event source.

use tachyon_protocol::Command;
use tachyon_types::ApprovalId;
use tokio::sync::mpsc::UnboundedSender;

use crate::state::{AppState, Confirm, Pane};

/// Reason recorded on a TUI-initiated denial (`Command::Deny` requires one).
pub const DENY_REASON: &str = "denied from the tachyon TUI";

/// One key as the input path sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Plain character (steering text, confirm answers).
    Char(char),
    /// Ctrl-modified character: every TUI command uses one of these so
    /// steering text never collides with a key binding.
    Ctrl(char),
    /// Enter: send the input line (or pick a task in the list).
    Enter,
    /// Escape: dismiss a confirmation or the graph overlay.
    Esc,
    /// Tab: next pane.
    Tab,
    /// Shift-Tab: previous pane.
    BackTab,
    /// Backspace: edit the input line.
    Backspace,
    /// Up arrow (task-list selection).
    Up,
    /// Down arrow (task-list selection).
    Down,
}

/// Everything the TUI can make happen, as one outbound message.
///
/// Nothing else leaves the input path: the render loop routes
/// [`Outbound::Command`] to the command connection, [`Outbound::Subscribe`]
/// to the reader, and treats [`Outbound::Quit`] as detach.
#[derive(Clone, Debug, PartialEq)]
pub enum Outbound {
    /// Strict request on the command connection.
    Command(tachyon_protocol::Command),
    /// Point the subscription connection at `task_id`, replaying from
    /// `after_seq` (D2 cursor rules, D3 task switch).
    Subscribe {
        /// Task to stream.
        task_id: tachyon_types::TaskId,
        /// Cursor the subscription resumes from.
        after_seq: i64,
    },
    /// Detach: close the TUI's connections. Never a task command.
    Quit,
}

/// Answers an armed confirmation (R1 board B6): `true` = the key was
/// consumed by the gate, whatever it did. Approve/deny re-check that the
/// armed id is still the pending ask before anything goes on the wire.
fn answer_confirm(state: &mut AppState, key: Key, out: &UnboundedSender<Outbound>) -> bool {
    let Some(confirm) = state.confirm.clone() else {
        return false;
    };
    match key {
        Key::Char('y') => {
            state.confirm = None;
            match confirm {
                Confirm::Cancel => {
                    if let Some(task_id) = state.attached_task {
                        let _ = out.send(Outbound::Command(Command::CancelTask { task_id }));
                    }
                }
                Confirm::Approve(id) => {
                    // The pending ask must still BE the armed one: a
                    // decided, expired or task-switched approval sends
                    // nothing (R1 board B6).
                    let still_pending = state
                        .pending
                        .as_ref()
                        .is_some_and(|pending| pending.id == id);
                    if still_pending
                        && let Some(task_id) = state.attached_task
                        && let Ok(approval_id) = id.parse::<ApprovalId>()
                    {
                        let _ = out.send(Outbound::Command(Command::Approve {
                            task_id,
                            approval_id,
                        }));
                    }
                }
                Confirm::Deny(id) => {
                    let still_pending = state
                        .pending
                        .as_ref()
                        .is_some_and(|pending| pending.id == id);
                    if still_pending
                        && let Some(task_id) = state.attached_task
                        && let Ok(approval_id) = id.parse::<ApprovalId>()
                    {
                        let _ = out.send(Outbound::Command(Command::Deny {
                            task_id,
                            approval_id,
                            reason: DENY_REASON.to_owned(),
                        }));
                    }
                }
            }
        }
        Key::Char('n') | Key::Esc => state.confirm = None,
        _ => {}
    }
    true
}

/// Renders one key onto `state`, pushing every effect onto `out`.
pub fn handle_key(state: &mut AppState, key: Key, out: &UnboundedSender<Outbound>) {
    // An armed confirmation captures every key: nothing types, nothing
    // sends, until the operator answers y / n / Esc.
    if answer_confirm(state, key, out) {
        return;
    }
    match key {
        Key::Ctrl('q' | 'c') => {
            let _ = out.send(Outbound::Quit);
        }
        // Picker: Enter with an empty line in the task list attaches.
        Key::Enter if state.input.is_empty() && state.pane == Pane::Tasks => {
            if let Some(row) = state.tasks.get(state.selected) {
                let task_id = row.id;
                // Wipe the previous task's display before replaying the
                // new one (D3 task switch — tested in input tests).
                state.reset_for(task_id);
                let _ = out.send(Outbound::Subscribe {
                    task_id,
                    after_seq: -1,
                });
            }
        }
        Key::Enter => {
            if state.input.is_empty() {
                return;
            }
            if let Some(task_id) = state.attached_task {
                let message = std::mem::take(&mut state.input);
                let _ = out.send(Outbound::Command(Command::SendMessage { task_id, message }));
            }
        }
        Key::Char(character) => state.input.push(character),
        Key::Backspace => {
            state.input.pop();
        }
        Key::Tab => state.pane = state.pane.next(),
        Key::BackTab => state.pane = state.pane.previous(),
        Key::Up if state.pane == Pane::Tasks && !state.tasks.is_empty() => {
            state.selected = state.selected.saturating_sub(1);
        }
        Key::Down if state.pane == Pane::Tasks && !state.tasks.is_empty() => {
            state.selected = (state.selected + 1).min(state.tasks.len() - 1);
        }
        Key::Ctrl('g') => {
            if state.graph {
                state.graph = false;
            } else {
                state.graph = true;
                if let Some(task_id) = state.attached_task {
                    let _ = out.send(Outbound::Command(Command::GetTask { task_id }));
                }
            }
        }
        Key::Esc if state.graph => state.graph = false,
        Key::Ctrl('p') => {
            if let Some(task_id) = state.attached_task {
                let _ = out.send(Outbound::Command(Command::PauseTask { task_id }));
            }
        }
        Key::Ctrl('r') => {
            if let Some(task_id) = state.attached_task {
                let _ = out.send(Outbound::Command(Command::ResumeTask { task_id }));
            }
        }
        Key::Ctrl('x') => {
            if state.attached_task.is_some() {
                state.confirm = Some(Confirm::Cancel);
            }
        }
        // Approve/deny arm a confirmation (R1 board B6): ^a/^d send
        // nothing by themselves; only the confirmed `y` decides the ask,
        // and only if it is still the armed pending id.
        Key::Ctrl('a') => {
            if state.attached_task.is_some()
                && let Some(pending) = state.pending.as_ref()
            {
                state.confirm = Some(Confirm::Approve(pending.id.clone()));
            }
        }
        Key::Ctrl('d') => {
            if state.attached_task.is_some()
                && let Some(pending) = state.pending.as_ref()
            {
                state.confirm = Some(Confirm::Deny(pending.id.clone()));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use tachyon_protocol::Command;
    use tachyon_types::TaskId;
    use tokio::sync::mpsc;

    use super::{DENY_REASON, Key, Outbound, handle_key};
    use crate::state::{AppState, Confirm, Pane, PendingApproval, TaskRow};

    fn drain(out: &mut mpsc::UnboundedReceiver<Outbound>) -> Vec<Outbound> {
        let mut sent = Vec::new();
        while let Ok(message) = out.try_recv() {
            sent.push(message);
        }
        sent
    }

    /// G3a input-mapping unit test: the detach key path must issue
    /// `Quit` and must never put a `CancelTask` on the outbound channel —
    /// closing the TUI does not cancel the task.
    #[test]
    fn detach_key_emits_quit_and_never_cancels_the_task() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let task = TaskId::generate();
        let mut state = AppState::new(Some(task));

        handle_key(&mut state, Key::Ctrl('q'), &tx);
        handle_key(&mut state, Key::Ctrl('c'), &tx);

        let sent = drain(&mut rx);
        let quits = sent
            .iter()
            .filter(|out| matches!(out, Outbound::Quit))
            .count();
        assert_eq!(quits, 2, "both detach keys must quit: {sent:?}");
        assert!(
            !sent
                .iter()
                .any(|out| matches!(out, Outbound::Command(Command::CancelTask { .. }))),
            "detach must never cancel the task: {sent:?}"
        );
    }

    /// Plan item 2/3: the input line is steering — Enter sends it as
    /// `Command::SendMessage` to the attached task and clears the line.
    #[test]
    fn enter_sends_the_input_line_as_a_steering_message_and_clears_it() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let task = TaskId::generate();
        let mut state = AppState::new(Some(task));

        for character in "look at foo()".chars() {
            handle_key(&mut state, Key::Char(character), &tx);
        }
        handle_key(&mut state, Key::Enter, &tx);

        let sent = drain(&mut rx);
        assert_eq!(
            sent,
            vec![Outbound::Command(Command::SendMessage {
                task_id: task,
                message: "look at foo()".to_owned(),
            })],
            "Enter must send exactly the input line as steering"
        );
        assert_eq!(state.input, "", "sending clears the input line");
    }

    /// Plan item 3: pause and resume keys send their task commands.
    #[test]
    fn pause_and_resume_keys_emit_their_task_commands() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let task = TaskId::generate();
        let mut state = AppState::new(Some(task));

        handle_key(&mut state, Key::Ctrl('p'), &tx);
        handle_key(&mut state, Key::Ctrl('r'), &tx);

        let sent = drain(&mut rx);
        assert_eq!(
            sent,
            vec![
                Outbound::Command(Command::PauseTask { task_id: task }),
                Outbound::Command(Command::ResumeTask { task_id: task }),
            ],
            "ctrl+p pauses and ctrl+r resumes the attached task"
        );
    }

    /// Plan item 3: cancel is confirm-gated — arming sends nothing, only
    /// the confirmed `y` puts a `CancelTask` on the channel, `n`/Esc abort.
    #[test]
    fn cancel_requires_confirmation_before_any_cancel_task_is_sent() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let task = TaskId::generate();
        let mut state = AppState::new(Some(task));

        handle_key(&mut state, Key::Ctrl('x'), &tx);
        assert_eq!(
            drain(&mut rx),
            vec![],
            "arming the cancel confirmation must send no command"
        );
        assert_eq!(
            state.confirm,
            Some(Confirm::Cancel),
            "confirmation is armed"
        );

        handle_key(&mut state, Key::Char('n'), &tx);
        assert_eq!(drain(&mut rx), vec![], "'n' aborts without sending");
        assert_eq!(state.confirm, None, "confirmation is dismissed");

        handle_key(&mut state, Key::Ctrl('x'), &tx);
        handle_key(&mut state, Key::Esc, &tx);
        assert_eq!(drain(&mut rx), vec![], "Esc aborts without sending");
        assert_eq!(state.confirm, None, "Esc dismisses the confirmation");

        handle_key(&mut state, Key::Ctrl('x'), &tx);
        handle_key(&mut state, Key::Char('y'), &tx);
        let sent = drain(&mut rx);
        assert_eq!(
            sent,
            vec![Outbound::Command(Command::CancelTask { task_id: task })],
            "only the confirmed 'y' cancels"
        );
        assert_eq!(state.confirm, None, "confirmation disarms after deciding");
    }

    /// R1 board B6: approve is confirm-gated like cancel — arming sends
    /// nothing, only the confirmed `y` puts an `Approve` on the channel,
    /// and only while the pending ask is still the armed id.
    #[test]
    fn approve_requires_confirmation_and_the_still_pending_id() {
        use tachyon_types::ApprovalId;

        let (tx, mut rx) = mpsc::unbounded_channel();
        let task = TaskId::generate();
        let mut state = AppState::new(Some(task));
        let approval = ApprovalId::generate();
        state.pending = Some(PendingApproval {
            id: approval.to_string(),
            request: "parked operation".to_owned(),
        });

        handle_key(&mut state, Key::Ctrl('a'), &tx);
        assert_eq!(
            drain(&mut rx),
            vec![],
            "arming the approve confirmation must send no Approve"
        );
        assert_eq!(
            state.confirm,
            Some(Confirm::Approve(approval.to_string())),
            "approve confirmation is armed with the pending id"
        );

        handle_key(&mut state, Key::Char('n'), &tx);
        assert_eq!(drain(&mut rx), vec![], "'n' aborts without sending");
        assert_eq!(state.confirm, None, "'n' disarms");

        handle_key(&mut state, Key::Ctrl('a'), &tx);
        handle_key(&mut state, Key::Char('y'), &tx);
        let sent = drain(&mut rx);
        assert_eq!(
            sent,
            vec![Outbound::Command(Command::Approve {
                task_id: task,
                approval_id: approval,
            })],
            "only the confirmed 'y' approves"
        );
        assert_eq!(state.confirm, None, "disarms after deciding");

        // Stale arm: the pending ask is gone (decided / task switched)
        // before `y` — the confirm consumes the key but sends nothing.
        handle_key(&mut state, Key::Ctrl('a'), &tx);
        state.pending = None;
        handle_key(&mut state, Key::Char('y'), &tx);
        assert_eq!(
            drain(&mut rx),
            vec![],
            "a cleared pending ask can never be approved by a stale confirm"
        );
        assert_eq!(state.confirm, None, "still disarms");
    }

    /// R1 board B6: deny is confirm-gated the same way, carrying the
    /// fixed TUI reason only on the confirmed send.
    #[test]
    fn deny_requires_confirmation_before_any_deny_is_sent() {
        use tachyon_types::ApprovalId;

        let (tx, mut rx) = mpsc::unbounded_channel();
        let task = TaskId::generate();
        let mut state = AppState::new(Some(task));
        let approval = ApprovalId::generate();
        state.pending = Some(PendingApproval {
            id: approval.to_string(),
            request: "parked operation".to_owned(),
        });

        handle_key(&mut state, Key::Ctrl('d'), &tx);
        assert_eq!(
            drain(&mut rx),
            vec![],
            "arming the deny confirmation must send no Deny"
        );
        assert_eq!(
            state.confirm,
            Some(Confirm::Deny(approval.to_string())),
            "deny confirmation is armed"
        );

        handle_key(&mut state, Key::Esc, &tx);
        assert_eq!(drain(&mut rx), vec![], "Esc aborts without sending");
        assert_eq!(state.confirm, None, "Esc disarms");

        handle_key(&mut state, Key::Ctrl('d'), &tx);
        handle_key(&mut state, Key::Char('y'), &tx);
        let sent = drain(&mut rx);
        assert_eq!(
            sent,
            vec![Outbound::Command(Command::Deny {
                task_id: task,
                approval_id: approval,
                reason: DENY_REASON.to_owned(),
            })],
            "only the confirmed 'y' denies"
        );
        assert_eq!(state.confirm, None, "disarms after deciding");
    }

    /// Plan item 3: while a confirmation is armed every other key is
    /// swallowed — it must not type into the steering line or slip past.
    #[test]
    fn keys_while_confirming_are_swallowed_not_typed() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let task = TaskId::generate();
        let mut state = AppState::new(Some(task));

        handle_key(&mut state, Key::Ctrl('x'), &tx);
        handle_key(&mut state, Key::Char('z'), &tx);
        handle_key(&mut state, Key::Enter, &tx);

        assert_eq!(drain(&mut rx), vec![], "no command may leak while armed");
        assert_eq!(state.input, "", "armed keys must not type into the line");
        assert_eq!(
            state.confirm,
            Some(Confirm::Cancel),
            "unrecognised keys keep the confirmation armed"
        );
    }

    /// Plan item 3: Tab / Shift-Tab cycle every pane, wrapping; the graph
    /// toggle fetches `GetTask` when opening (inspection fed from `GetTask`)
    /// and Escape closes it without another fetch.
    #[test]
    fn tab_cycles_panes_and_ctrl_g_toggles_graph_via_get_task() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let task = TaskId::generate();
        let mut state = AppState::new(Some(task));
        assert_eq!(state.pane, Pane::Conversation);

        handle_key(&mut state, Key::Tab, &tx);
        assert_eq!(state.pane, Pane::Tasks);
        handle_key(&mut state, Key::Tab, &tx);
        assert_eq!(state.pane, Pane::Operations);
        handle_key(&mut state, Key::Tab, &tx);
        assert_eq!(state.pane, Pane::Files);
        handle_key(&mut state, Key::Tab, &tx);
        assert_eq!(state.pane, Pane::Approvals);
        handle_key(&mut state, Key::Tab, &tx);
        assert_eq!(
            state.pane,
            Pane::Conversation,
            "Tab wraps back to the conversation"
        );
        handle_key(&mut state, Key::BackTab, &tx);
        assert_eq!(
            state.pane,
            Pane::Approvals,
            "Shift-Tab cycles backwards, wrapping"
        );
        assert_eq!(drain(&mut rx), vec![], "pane switching sends nothing");

        handle_key(&mut state, Key::Ctrl('g'), &tx);
        assert!(state.graph, "ctrl+g opens the graph overlay");
        assert_eq!(
            drain(&mut rx),
            vec![Outbound::Command(Command::GetTask { task_id: task })],
            "opening the graph fetches GetTask"
        );
        handle_key(&mut state, Key::Esc, &tx);
        assert!(!state.graph, "Escape closes the graph overlay");
        assert_eq!(drain(&mut rx), vec![], "closing fetches nothing");
    }

    /// Plan item 4 picker support: Enter in the task list attaches —
    /// `Subscribe` from a full-replay cursor (-1) on the reader connection,
    /// never a command-connection frame (D3).
    #[test]
    fn enter_on_the_task_list_attaches_the_subscription_to_the_selected_task() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let first = TaskId::generate();
        let second = TaskId::generate();
        let mut state = AppState::new(None);
        assert_eq!(state.pane, Pane::Tasks, "no task starts on the picker");
        state.tasks = vec![
            TaskRow {
                id: first,
                status: "Created".to_owned(),
                objective: "first".to_owned(),
                revision: 0,
            },
            TaskRow {
                id: second,
                status: "Paused".to_owned(),
                objective: "second".to_owned(),
                revision: 3,
            },
        ];

        handle_key(&mut state, Key::Down, &tx);
        assert_eq!(state.selected, 1, "Down moves the selection");
        handle_key(&mut state, Key::Enter, &tx);

        let sent = drain(&mut rx);
        assert_eq!(
            sent,
            vec![Outbound::Subscribe {
                task_id: second,
                after_seq: -1,
            }],
            "Enter subscribes the reader from a full-replay cursor"
        );
        assert_eq!(
            state.attached_task,
            Some(second),
            "selection becomes the attached task"
        );
        assert_eq!(
            state.pane,
            Pane::Conversation,
            "attaching jumps to the conversation"
        );

        // Selection is bounded: it never runs off the end of the list.
        handle_key(&mut state, Key::Down, &tx);
        handle_key(&mut state, Key::Down, &tx);
        state.pane = Pane::Tasks;
        handle_key(&mut state, Key::Down, &tx);
        assert_eq!(state.selected, 1, "selection clamps to the last row");
        handle_key(&mut state, Key::Up, &tx);
        handle_key(&mut state, Key::Up, &tx);
        assert_eq!(state.selected, 0, "selection clamps at the first row");
        assert_eq!(drain(&mut rx), vec![], "moving the selection sends nothing");
    }

    /// Plan item 3 (R1 board B6): with a pending approval, ctrl+a /
    /// ctrl+d ARM their confirmations — bare keys never send; with
    /// nothing pending they do not even arm. The confirmed sends are
    /// covered by the approve/deny confirmation tests above.
    #[test]
    fn approval_keys_arm_or_stay_silent_never_send_bare() {
        use tachyon_types::ApprovalId;

        let (tx, mut rx) = mpsc::unbounded_channel();
        let task = TaskId::generate();
        let approval = ApprovalId::generate();
        let mut state = AppState::new(Some(task));

        handle_key(&mut state, Key::Ctrl('a'), &tx);
        handle_key(&mut state, Key::Ctrl('d'), &tx);
        assert_eq!(
            drain(&mut rx),
            vec![],
            "no pending approval means no decision commands"
        );
        assert_eq!(state.confirm, None, "and nothing arms");

        state.pending = Some(PendingApproval {
            id: approval.to_string(),
            request: "shell: rm build-cache".to_owned(),
        });
        handle_key(&mut state, Key::Ctrl('a'), &tx);
        assert_eq!(drain(&mut rx), vec![], "bare ^a never sends");
        assert_eq!(
            state.confirm,
            Some(Confirm::Approve(approval.to_string())),
            "^a arms the approve confirmation"
        );
        handle_key(&mut state, Key::Char('n'), &tx);

        handle_key(&mut state, Key::Ctrl('d'), &tx);
        assert_eq!(drain(&mut rx), vec![], "bare ^d never sends");
        assert_eq!(
            state.confirm,
            Some(Confirm::Deny(approval.to_string())),
            "^d arms the deny confirmation"
        );
        assert!(!DENY_REASON.is_empty(), "a denial records a reason");
    }

    /// Plan item 3 input line: Backspace edits the steering text.
    #[test]
    fn backspace_edits_the_input_line() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let task = TaskId::generate();
        let mut state = AppState::new(Some(task));

        for character in "hell".chars() {
            handle_key(&mut state, Key::Char(character), &tx);
        }
        handle_key(&mut state, Key::Backspace, &tx);
        assert_eq!(state.input, "hel", "Backspace removes the last character");

        for _ in 0..5 {
            handle_key(&mut state, Key::Backspace, &tx);
        }
        assert_eq!(
            state.input, "",
            "Backspace on an empty line stays empty, no panic"
        );
        assert_eq!(drain(&mut rx), vec![], "editing sends nothing");
    }

    /// D3 task switch: attaching a different task from the picker wipes
    /// the previous task's display so replay starts from a clean state —
    /// no frame of the old task may survive into the new attach.
    #[test]
    fn picking_a_new_task_wipes_the_previous_tasks_display() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let first = TaskId::generate();
        let second = TaskId::generate();
        let mut state = AppState::new(Some(first));

        // Display state accumulated while attached to `first`.
        state.status = Some("Paused".to_owned());
        state.objective = Some("first objective".to_owned());
        state.revision = Some(7);
        state.conversation.push("steering: old note".to_owned());
        state.operations.push("stage: evidence → model".to_owned());
        state.files.push("changed: old.rs".to_owned());
        state.approvals.push("granted approval x: ok".to_owned());
        state.last_applied_seq = 42;
        state.notice = Some("stale".to_owned());
        state.tasks = vec![TaskRow {
            id: second,
            status: "Created".to_owned(),
            objective: "second objective".to_owned(),
            revision: 0,
        }];

        // Back to the picker, select the other task, attach.
        handle_key(&mut state, Key::Tab, &tx);
        handle_key(&mut state, Key::Enter, &tx);

        let sent = drain(&mut rx);
        assert!(
            sent.contains(&Outbound::Subscribe {
                task_id: second,
                after_seq: -1,
            }),
            "the switch subscribes from a full-replay cursor: {sent:?}"
        );
        assert_eq!(state.attached_task, Some(second));
        assert_eq!(state.status, None, "old status wiped");
        assert_eq!(state.objective, None, "old objective wiped");
        assert_eq!(state.revision, None, "old revision wiped");
        assert_eq!(
            state.conversation,
            Vec::<String>::new(),
            "old conversation wiped"
        );
        assert_eq!(state.operations, Vec::<String>::new(), "old feed wiped");
        assert_eq!(state.files, Vec::<String>::new(), "old receipts wiped");
        assert_eq!(state.approvals, Vec::<String>::new(), "old approvals wiped");
        assert_eq!(state.notice, None, "old notice wiped");
        assert_eq!(
            state.last_applied_seq, -1,
            "the cursor display resets so replay can start over"
        );
        assert_eq!(
            state.pane,
            Pane::Conversation,
            "attaching jumps to the conversation"
        );
    }
}
