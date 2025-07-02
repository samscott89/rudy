//! Test case for understanding DWARF structure of different Rust method types
//! This will help us optimize method discovery by understanding how methods
//! are organized in the debug information.

use std::fmt;

// A simple struct to test methods on
pub struct Session {
    id: u64,
    name: String,
}

// Regular impl block with methods
impl Session {
    // Constructor method
    pub fn new(id: u64, name: String) -> Self {
        Session { id, name }
    }

    // Method taking &self
    pub fn get_id(&self) -> u64 {
        self.id
    }

    // Method taking &mut self
    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    // Method taking self (consuming)
    pub fn into_name(self) -> String {
        self.name
    }

    // Associated function (no self)
    pub fn default_name() -> &'static str {
        "default_session"
    }
}

// Trait definition
pub trait Describable {
    fn describe(&self) -> String;
    fn description_length(&self) -> usize;
}

// Trait implementation for Session
impl Describable for Session {
    fn describe(&self) -> String {
        format!("Session {} with name: {}", self.id, self.name)
    }

    fn description_length(&self) -> usize {
        self.describe().len()
    }
}

// Another trait to test multiple trait impls
pub trait Identifiable {
    fn identifier(&self) -> String;
}

impl Identifiable for Session {
    fn identifier(&self) -> String {
        format!("session_{}", self.id)
    }
}

// Implement a standard library trait
impl fmt::Display for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Session(id={}, name={})", self.id, self.name)
    }
}

// Generic struct to test generic methods
pub struct Container<T> {
    value: T,
}

impl<T> Container<T> {
    pub fn new(value: T) -> Self {
        Container { value }
    }

    pub fn get(&self) -> &T {
        &self.value
    }
}

// Specific implementation for Container<Session>
impl Container<Session> {
    pub fn get_session_id(&self) -> u64 {
        self.value.id
    }
}

// Free functions that work with Session
pub fn create_session(id: u64) -> Session {
    Session::new(id, "created".to_string())
}

pub fn process_session(session: &Session) -> String {
    format!("Processing: {}", session.describe())
}

pub fn modify_session(session: &mut Session, new_name: String) {
    session.set_name(new_name);
}

// Nested module to test path resolution
pub mod utils {
    use super::{Identifiable, Session};

    pub fn validate_session(session: &Session) -> bool {
        session.get_id() > 0
    }

    pub fn format_session(session: &Session) -> String {
        format!("Session: {}", session.identifier())
    }
}

// A function to ensure everything gets compiled and linked
pub fn test_all_methods(mut session: Session) {
    // Call regular methods
    let _ = session.get_id();
    session.set_name("updated".to_string());

    // Call trait methods (need to import traits)
    let _ = session.describe(); // Describable trait
    let _ = session.description_length(); // Describable trait
    let _ = session.identifier(); // Identifiable trait
    let _ = format!("{session}"); // Display trait

    // Call free functions
    let _ = process_session(&session);
    modify_session(&mut session, "modified".to_string());

    // Call nested module functions
    let _ = utils::validate_session(&session);
    let _ = utils::format_session(&session);

    // Generic methods
    let container = Container::new(session);
    let _ = container.get();
    let _ = container.get_session_id();
}

fn main() {
    let session = Session::new(1, "test".to_string());
    test_all_methods(session);
    println!("Method discovery test completed");
}
