/// A response is associated with the refresh request that issued it.
#[derive(Clone, Copy, Debug)]
pub struct RefreshTicket {
    generation: u64,
}

/// Synthetic session state. Completion order can differ from request order.
#[derive(Debug)]
pub struct Session {
    issued_generation: u64,
    active_generation: u64,
    token: String,
}

impl Session {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            issued_generation: 0,
            active_generation: 0,
            token: token.into(),
        }
    }

    pub fn begin_refresh(&mut self) -> RefreshTicket {
        self.issued_generation = self
            .issued_generation
            .checked_add(1)
            .expect("generation exhausted");
        RefreshTicket {
            generation: self.issued_generation,
        }
    }

    pub fn complete_refresh(&mut self, ticket: RefreshTicket, token: impl Into<String>) {
        if ticket.generation > self.active_generation {
            self.active_generation = ticket.generation;
            self.token = token.into();
        }
    }

    pub fn credential(&self) -> &str {
        &self.token
    }
    pub fn generation(&self) -> u64 {
        self.active_generation
    }
}
