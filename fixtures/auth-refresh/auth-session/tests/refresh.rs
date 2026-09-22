use auth_session::Session;

#[test]
fn out_of_order_completions_keep_newest_token() {
    let mut session = Session::new("synthetic-initial");
    let earlier = session.begin_refresh();
    let later = session.begin_refresh();
    // Deterministic delivery order: request 2 completes BEFORE request 1.
    session.complete_refresh(later, "synthetic-current");
    session.complete_refresh(earlier, "synthetic-obsolete");
    assert_eq!(
        session.generation(),
        2,
        "stale refresh regressed the generation"
    );
    assert_eq!(session.credential(), "synthetic-current");
}

#[test]
fn duplicate_completion_cannot_replace_an_accepted_token() {
    let mut session = Session::new("synthetic-initial");
    let ticket = session.begin_refresh();
    session.complete_refresh(ticket, "synthetic-current");
    session.complete_refresh(ticket, "synthetic-duplicate");
    assert_eq!(session.credential(), "synthetic-current");
}

#[test]
fn newer_completion_updates_the_session() {
    let mut session = Session::new("synthetic-initial");
    let ticket = session.begin_refresh();
    session.complete_refresh(ticket, "synthetic-current");
    assert_eq!(session.generation(), 1);
    assert_eq!(session.credential(), "synthetic-current");
}
