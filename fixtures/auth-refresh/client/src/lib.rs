use auth_session::Session;

pub fn foreground_credential(session: &Session) -> &str {
    session.credential()
}
pub fn retry_credential(session: &Session) -> String {
    session.credential().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_call_sites_observe_the_latest_completed_generation() {
        let mut session = Session::new("synthetic-initial");
        let earlier = session.begin_refresh();
        let later = session.begin_refresh();
        session.complete_refresh(later, "synthetic-current");
        session.complete_refresh(earlier, "synthetic-obsolete");
        assert_eq!(foreground_credential(&session), "synthetic-current");
        assert_eq!(retry_credential(&session), "synthetic-current");
    }
}
