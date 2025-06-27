//! Demo program to test LLDB integration
//!
//! Compile with: rustc -g examples/lldb_demo.rs -o target/debug/lldb_demo

use std::collections::HashMap;

#[allow(unused)]
#[derive(Debug)]
struct User {
    id: u64,
    name: String,
    email: String,
    metadata: HashMap<String, String>,
}

#[allow(unused)]
#[derive(Debug)]
struct Session {
    user: User,
    token: String,
    expires_at: u64,
}

impl User {
    fn new(id: u64, name: &str, email: &str) -> Self {
        let mut metadata = HashMap::new();
        metadata.insert("created".to_string(), "2024-01-01".to_string());
        metadata.insert("role".to_string(), "admin".to_string());

        User {
            id,
            name: name.to_string(),
            email: email.to_string(),
            metadata,
        }
    }
}

fn create_session(user: User) -> Session {
    Session {
        user,
        token: "secret-token-12345".to_string(),
        expires_at: 1234567890,
    }
}

fn main() {
    let user = User::new(42, "Alice Smith", "alice@example.com");
    let session = create_session(user);

    // Set a breakpoint here to inspect with LLDB
    println!("Session created: {:?}", session);

    // Create some more complex data
    let mut users = Vec::new();
    for i in 0..5 {
        users.push(User::new(
            i,
            &format!("User {}", i),
            &format!("user{}@example.com", i),
        ));
    }

    println!("Created {} users", users.len());
}
