# AI Development Slop Review Guide

**Date:** 2026-04-06
**Scope:** Rust codebase review checklist for detecting AI-generated code quality issues
**Applies to:** `grokrs` workspace and all `grokrs-*` crates

---

## What Is "AI Slop"?

AI slop is code that compiles, passes basic checks, and looks plausible -- but is subtly wrong,
bloated, or misaligned with the codebase it lives in. The term parallels "AI slop" in content
generation: machine-produced output that is superficially acceptable but lacks the intentionality,
precision, and contextual awareness of carefully authored work.

AI slop is dangerous precisely because it *passes the eye test*. It does not look broken. It looks
like real code written by a competent developer. The problems hide in the places reviewers skim:
edge cases, error paths, architectural assumptions, and performance characteristics.

### Key Research Findings

| Finding | Source |
|---|---|
| AI-co-authored PRs contain 1.7x more issues than human-only PRs | CodeRabbit, 470 PRs |
| Copy/paste code rose from 8.3% to 12.3%; refactoring dropped from 25% to <10% | GitClear, 211M lines |
| 36-40% of AI-generated code snippets contain security vulnerabilities | Snyk Research |
| 90-100% of AI-generated repos exhibit "comments everywhere" anti-pattern | OX Security, 300 projects |
| Logic errors 75% more common, readability issues 3x worse in AI PRs | CodeRabbit |
| Performance regressions related to excessive I/O are 8x more common in AI code | CodeRabbit |

### Why Rust Is Both Helped and Hurt

Rust's compiler catches entire classes of bugs (memory safety, data races, null) that AI slop
would introduce in other languages. However, the compiler cannot catch:

- Idiomatic misuse (compiles but is wasteful or non-Rustic)
- Architectural violations (wrong crate boundary, wrong abstraction level)
- Performance pathologies (unnecessary allocations, clones, copies)
- Semantic incorrectness (wrong logic that type-checks)
- Missing invariants (code that should enforce rules but does not)

The compiler is a filter, not a guarantee. Everything in this guide targets what gets *through*
that filter.

---

## How to Use This Guide

For each pattern below:

1. **Name** -- a memorable label for the anti-pattern
2. **Why It Is Slop** -- the actual harm, not just "it is bad"
3. **Bad Example** -- Rust code demonstrating the pattern
4. **Correct Approach** -- idiomatic Rust replacement
5. **Detection** -- grep, clippy, or manual review command

During code review, scan for these patterns in priority order within each category. Patterns
marked with `[AUTO]` can be caught by tooling. Patterns marked with `[MANUAL]` require human
judgment.

---

## Category 1: Safety Slop

### Pattern 1: Unwrap Abuse `[AUTO]`

**Why it is slop:** `.unwrap()` converts a recoverable error into a panic. AI uses it as a
shortcut to avoid thinking about error paths. In library code or any production path, a panic
is a crash. In async server code, it tears down the task or the entire runtime. AI reaches for
`.unwrap()` because training data is full of examples, tutorials, and prototypes that use it.

**Bad:**
```rust
fn load_config(path: &str) -> AppConfig {
    let contents = std::fs::read_to_string(path).unwrap();
    toml::from_str(&contents).unwrap()
}
```

**Correct:**
```rust
fn load_config(path: &str) -> Result<AppConfig, ConfigError> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| ConfigError::Io { path: path.into(), source: e })?;
    toml::from_str(&contents)
        .map_err(|e| ConfigError::Parse { path: path.into(), source: e })
}
```

**Detection:**
```bash
# Clippy lint (deny in CI)
cargo clippy -- -D clippy::unwrap_used -D clippy::expect_used

# Grep for manual inspection
grep -rn '\.unwrap()' crates/ --include='*.rs' | grep -v '#\[cfg(test)\]' | grep -v '/tests/'
grep -rn '\.expect(' crates/ --include='*.rs' | grep -v '#\[cfg(test)\]' | grep -v '/tests/'
```

---

### Pattern 2: Expect With Useless Messages `[AUTO]`

**Why it is slop:** AI often "fixes" unwrap by adding `.expect("failed")` -- which is just an
unwrap with a string that adds no diagnostic value. A good expect message should tell you *what*
failed and *why* it should be impossible, not just restate that something went wrong.

**Bad:**
```rust
let port = env::var("PORT").expect("failed to get PORT");
let port: u16 = port.parse().expect("failed to parse");
```

**Correct:**
```rust
// If this truly cannot fail (e.g., compile-time guaranteed), document why:
let port = env::var("PORT")
    .expect("PORT must be set -- validated at startup in main()");
let port: u16 = port.parse()
    .expect("PORT validated as numeric by config loader");

// If it CAN fail, propagate:
let port: u16 = env::var("PORT")
    .map_err(|_| ConfigError::Missing("PORT"))?
    .parse()
    .map_err(|_| ConfigError::Invalid("PORT", "must be a valid u16"))?;
```

**Detection:**
```bash
# Find short/generic expect messages (likely AI slop)
grep -rn '\.expect("' crates/ --include='*.rs' | grep -E '\.expect\("(failed|error|should|invalid|bad|could not|unable)'
```

---

### Pattern 3: Unsafe Without Justification `[MANUAL]`

**Why it is slop:** AI sometimes reaches for `unsafe` to bypass borrow checker errors instead of
fixing the design. In Rust, `unsafe` is a contract: you are telling the compiler "I have manually
verified these invariants." AI has verified nothing. Even when unsafe is warranted, AI rarely
provides the required safety comment explaining *why* the invariants hold.

**Bad:**
```rust
fn get_item<'a>(items: &'a [Item], index: usize) -> &'a Item {
    unsafe { items.get_unchecked(index) }
}
```

**Correct:**
```rust
fn get_item(items: &[Item], index: usize) -> Option<&Item> {
    items.get(index)
}

// If unsafe is truly needed (proven hot path, bounds already checked):
fn get_item_unchecked(items: &[Item], index: usize) -> &Item {
    debug_assert!(index < items.len(), "index {index} out of bounds for len {}", items.len());
    // SAFETY: caller guarantees index < items.len() via prior bounds check
    // in the tight loop at src/engine.rs:142. Verified by property test in
    // tests/engine_bounds.rs.
    unsafe { items.get_unchecked(index) }
}
```

**Detection:**
```bash
# Find all unsafe blocks
grep -rn 'unsafe {' crates/ --include='*.rs'
# Check that each has a preceding // SAFETY: comment
grep -B2 'unsafe {' crates/ --include='*.rs' | grep -v 'SAFETY:'
```

---

### Pattern 4: Silent Error Swallowing `[AUTO]`

**Why it is slop:** AI wraps operations in match/if-let and silently discards the error branch.
The code compiles and appears to "handle" errors, but actually drops critical failure information.
This turns detectable failures into silent data corruption or mysterious misbehavior.

**Bad:**
```rust
fn save_record(db: &Database, record: &Record) {
    if let Ok(_) = db.insert(record) {
        log::info!("saved record");
    }
    // Error case: nothing. Record silently not saved.
}
```

**Correct:**
```rust
fn save_record(db: &Database, record: &Record) -> Result<(), StoreError> {
    db.insert(record).map_err(|e| {
        log::error!(record_id = %record.id, error = %e, "failed to save record");
        StoreError::Insert { id: record.id, source: e }
    })?;
    log::info!(record_id = %record.id, "saved record");
    Ok(())
}
```

**Detection:**
```bash
# Find if-let-ok patterns that discard errors
grep -rn 'if let Ok(' crates/ --include='*.rs'
# Find match arms that use _ => {} for errors
grep -rn 'Err(_)' crates/ --include='*.rs' | grep -E '(\{\}|=>\s*\(\))'
# Clippy lint
cargo clippy -- -D clippy::let_underscore_must_use
```

---

### Pattern 5: Index-Based Access Without Bounds Checking `[AUTO]`

**Why it is slop:** AI freely indexes into slices and vectors with `items[i]` instead of using
`.get(i)`. This compiles but panics at runtime on out-of-bounds access. AI does this because
indexing is simpler than handling the `Option` from `.get()`.

**Bad:**
```rust
fn first_and_last(items: &[String]) -> (String, String) {
    (items[0].clone(), items[items.len() - 1].clone())
}
```

**Correct:**
```rust
fn first_and_last(items: &[String]) -> Option<(&str, &str)> {
    let first = items.first()?;
    let last = items.last()?;
    Some((first.as_str(), last.as_str()))
}
```

**Detection:**
```bash
cargo clippy -- -D clippy::indexing_slicing
```

---

## Category 2: Performance Slop

### Pattern 6: Unnecessary Clone Everywhere `[AUTO]`

**Why it is slop:** AI's primary strategy for satisfying the borrow checker is to clone everything.
This bypasses ownership thinking entirely. Each `.clone()` on a `String`, `Vec`, or complex struct
is a heap allocation. In hot paths, this turns Rust's zero-cost abstractions into Java-level
garbage generation. The clone "works" but defeats the purpose of using Rust.

**Bad:**
```rust
fn process_names(users: &[User]) -> Vec<String> {
    let mut results = Vec::new();
    for user in users {
        let name = user.name.clone(); // Unnecessary: we only need to read it
        let upper = name.to_uppercase();
        results.push(upper);
    }
    results
}
```

**Correct:**
```rust
fn process_names(users: &[User]) -> Vec<String> {
    users.iter()
        .map(|user| user.name.to_uppercase()) // No clone needed
        .collect()
}
```

**Detection:**
```bash
cargo clippy -- -D clippy::redundant_clone -D clippy::clone_on_copy

# Manual audit: find all .clone() calls and verify each is necessary
grep -rn '\.clone()' crates/ --include='*.rs' | grep -v '/tests/' | wc -l
```

---

### Pattern 7: String Instead of &str in Function Parameters `[AUTO]`

**Why it is slop:** AI defaults to `String` parameters because it avoids lifetime thinking. This
forces every caller to allocate a new `String` even when they have a `&str` available. It is the
Rust equivalent of Java's "pass everything by value" -- technically works, wastes memory and CPU
on every call.

**Bad:**
```rust
fn find_user(name: String) -> Option<User> {
    users.iter().find(|u| u.name == name).cloned()
}

// Caller is forced to allocate:
let user = find_user("alice".to_string());
```

**Correct:**
```rust
fn find_user(name: &str) -> Option<&User> {
    users.iter().find(|u| u.name == name)
}

// Caller can pass &str directly:
let user = find_user("alice");
```

**Detection:**
```bash
# Find functions taking String where &str would suffice
grep -rn 'fn .*\(.*: String' crates/ --include='*.rs' | grep -v 'pub struct\|pub enum'

# Clippy lint for the specific case of &String parameters
cargo clippy -- -D clippy::ptr_arg
```

---

### Pattern 8: Collecting When You Could Iterate `[AUTO]`

**Why it is slop:** AI loves to `.collect()` into a `Vec` mid-pipeline, then immediately iterate
over it again. This creates an intermediate allocation that serves no purpose. Iterators in Rust
are lazy and zero-cost -- collecting them prematurely throws away that advantage.

**Bad:**
```rust
fn count_active(users: &[User]) -> usize {
    let active_users: Vec<&User> = users.iter()
        .filter(|u| u.is_active)
        .collect(); // Pointless intermediate Vec
    active_users.len()
}
```

**Correct:**
```rust
fn count_active(users: &[User]) -> usize {
    users.iter().filter(|u| u.is_active).count()
}
```

**Detection:**
```bash
# Find collect-then-len or collect-then-iter patterns
grep -rn '\.collect::<Vec' crates/ --include='*.rs'
cargo clippy -- -D clippy::needless_collect
```

---

### Pattern 9: Repeated Allocation in Loops `[MANUAL]`

**Why it is slop:** AI allocates new `String`s, `Vec`s, or `HashMap`s inside loops instead of
reusing a buffer. The compiler cannot optimize this away. In tight loops, this generates
thousands of allocations that a pre-allocated buffer would eliminate.

**Bad:**
```rust
fn process_lines(lines: &[&str]) -> Vec<String> {
    let mut results = Vec::new();
    for line in lines {
        let mut parts = Vec::new(); // Allocated every iteration
        for word in line.split_whitespace() {
            let processed = String::new(); // Allocated every iteration
            // ... processing ...
            parts.push(processed);
        }
        results.push(parts.join(", "));
    }
    results
}
```

**Correct:**
```rust
fn process_lines(lines: &[&str]) -> Vec<String> {
    let mut results = Vec::with_capacity(lines.len());
    let mut parts = Vec::new(); // Reuse across iterations
    let mut buf = String::new(); // Reuse across iterations
    for line in lines {
        parts.clear();
        for word in line.split_whitespace() {
            buf.clear();
            // ... processing into buf ...
            parts.push(buf.clone()); // Clone only when storing
        }
        results.push(parts.join(", "));
    }
    results
}
```

**Detection:**
```bash
# Find Vec::new() or String::new() inside loop bodies
# Requires manual inspection -- grep for patterns then review context
grep -rn 'Vec::new()\|String::new()\|HashMap::new()' crates/ --include='*.rs'
```

---

### Pattern 10: Formatting Strings for Non-Display Purposes `[AUTO]`

**Why it is slop:** AI uses `format!()` to build strings that are immediately compared, hashed,
or converted to something else. Each `format!()` allocates a new `String`. When used in
comparisons or map keys, this is pure waste.

**Bad:**
```rust
fn has_permission(user: &User, resource: &str) -> bool {
    let key = format!("{}:{}", user.role, resource);
    PERMISSIONS.contains(&key.as_str())
}
```

**Correct:**
```rust
fn has_permission(user: &User, resource: &str) -> bool {
    PERMISSIONS.iter().any(|p| {
        p.starts_with(user.role) && p.ends_with(resource)
    })
    // Or use a tuple key: (role, resource) in a HashSet
}
```

**Detection:**
```bash
# Find format! used in comparisons or as hash keys
grep -rn 'format!' crates/ --include='*.rs' | grep -E '\.(contains|get|insert|==)'
cargo clippy -- -D clippy::format_in_format_args
```

---

## Category 3: Structure Slop

### Pattern 11: God Functions `[MANUAL]`

**Why it is slop:** AI generates monolithic functions that fetch, validate, transform, persist,
and notify all in one block. These are impossible to unit test in isolation, difficult to reason
about, and violate single responsibility at every level. AI does this because it implements
prompts directly without considering decomposition.

**Bad:**
```rust
async fn handle_request(req: Request) -> Response {
    // Validate input (20 lines)
    // Fetch from database (15 lines)
    // Transform data (25 lines)
    // Call external API (10 lines)
    // Save results (15 lines)
    // Send notification (10 lines)
    // Build response (10 lines)
    // ... 100+ lines total
}
```

**Correct:**
```rust
async fn handle_request(req: Request) -> Result<Response, ApiError> {
    let input = validate_input(&req)?;
    let record = fetch_record(&db, input.id).await?;
    let transformed = transform(record, &input.params)?;
    let result = call_external(&client, &transformed).await?;
    save_result(&db, &result).await?;
    notify(&notifier, &result).await?;
    Ok(build_response(result))
}
```

**Detection:**
```bash
# Find functions over 50 lines (rough heuristic)
# Use cargo-bloat or manual review
grep -c 'fn ' crates/*/src/**/*.rs  # Compare function count to file length

# Clippy has a configurable lint:
# In clippy.toml: too-many-lines-threshold = 50
cargo clippy -- -W clippy::too_many_lines
```

---

### Pattern 12: Copy-Paste Duplication `[AUTO]`

**Why it is slop:** AI does not search the codebase before generating code. It generates what
looks correct based on the current context window. If a shared utility exists three directories
away, the AI will reimplement it. GitClear found copy/paste code rose from 8.3% to 12.3% of
all changed lines across 211 million lines analyzed. Each duplicate is a maintenance burden
and a consistency risk.

**Bad:**
```rust
// In crates/grokrs-api/src/client.rs:
fn validate_api_key(key: &str) -> bool {
    !key.is_empty() && key.starts_with("xai-") && key.len() >= 20
}

// In crates/grokrs-cli/src/config.rs (independently reimplemented):
fn check_api_key(api_key: &str) -> bool {
    api_key.starts_with("xai-") && api_key.len() > 15 && !api_key.is_empty()
}
```

**Correct:**
```rust
// In crates/grokrs-core/src/validation.rs (single source of truth):
pub fn validate_api_key(key: &str) -> Result<(), ConfigError> {
    if key.is_empty() {
        return Err(ConfigError::Validation("API key is empty".into()));
    }
    if !key.starts_with("xai-") {
        return Err(ConfigError::Validation("API key must start with 'xai-'".into()));
    }
    if key.len() < 20 {
        return Err(ConfigError::Validation("API key too short".into()));
    }
    Ok(())
}
```

**Detection:**
```bash
# Find suspiciously similar function signatures
grep -rn 'fn.*validate\|fn.*check\|fn.*verify' crates/ --include='*.rs' | sort

# Use cargo-machete for unused dependencies (related: AI adds deps it does not use)
cargo install cargo-machete && cargo machete
```

---

### Pattern 13: Abstraction for a Single Implementation `[MANUAL]`

**Why it is slop:** AI over-engineers by creating traits, trait objects, and generic frameworks
for code that has exactly one concrete implementation. This adds indirection without benefit.
You get `trait Storage`, `struct SqliteStorage`, and a generic `Repository<S: Storage>` when
the project only ever uses SQLite. The abstraction makes the code harder to navigate, harder
to debug, and impossible to optimize through.

**Bad:**
```rust
trait MessageFormatter {
    fn format(&self, msg: &str) -> String;
}

struct DefaultFormatter;

impl MessageFormatter for DefaultFormatter {
    fn format(&self, msg: &str) -> String {
        format!("[INFO] {msg}")
    }
}

fn log_message(formatter: &dyn MessageFormatter, msg: &str) {
    println!("{}", formatter.format(msg));
}
```

**Correct:**
```rust
fn format_log_message(msg: &str) -> String {
    format!("[INFO] {msg}")
}

fn log_message(msg: &str) {
    println!("{}", format_log_message(msg));
}
// Add the trait WHEN a second implementation actually exists.
```

**Detection:**
```bash
# Find traits with only one impl
grep -rn '^pub trait ' crates/ --include='*.rs'
# Then for each, check how many impl blocks exist:
grep -rn 'impl SomeTraitName for' crates/ --include='*.rs'
```

---

### Pattern 14: Debugging Residue and Variant Files `[AUTO]`

**Why it is slop:** AI agents iterate by creating variant files during debugging. They do not
clean up after themselves. You end up with `client.rs`, `client_v2.rs`, `client_new.rs` in
the same directory. Each variant was briefly used during the AI's trial-and-error process.
None should reach version control.

**Bad:** Files like `config_old.rs`, `api_v2.rs`, `handler_backup.rs`, `utils_temp.rs`

**Correct:** One canonical file per module. If you need history, that is what version control is for.

**Detection:**
```bash
# Find variant/residue files
find crates/ -name '*_old.rs' -o -name '*_v2.rs' -o -name '*_v3.rs' \
    -o -name '*_new.rs' -o -name '*_backup.rs' -o -name '*_temp.rs' \
    -o -name '*_fixed.rs' -o -name '*_wip.rs'
```

---

## Category 4: Idiomatic Slop

### Pattern 15: Java-in-Rust (OOP Cargo Cult) `[MANUAL]`

**Why it is slop:** AI trained on Java/C# produces Rust that looks like Java with different
syntax. Getters and setters on every field. Builder patterns where a simple struct literal
would suffice. Inheritance hierarchies emulated through trait objects. This makes the code
verbose, unfamiliar to Rust developers, and misses Rust's strengths (algebraic types, pattern
matching, ownership).

**Bad:**
```rust
pub struct User {
    name: String,
    age: u32,
}

impl User {
    pub fn new() -> Self {
        Self { name: String::new(), age: 0 }
    }
    pub fn get_name(&self) -> &str { &self.name }
    pub fn set_name(&mut self, name: String) { self.name = name; }
    pub fn get_age(&self) -> u32 { self.age }
    pub fn set_age(&mut self, age: u32) { self.age = age; }
}
```

**Correct:**
```rust
pub struct User {
    pub name: String,
    pub age: u32,
}

// Or if invariants need protection:
pub struct User {
    name: String,
    age: u32,
}

impl User {
    pub fn new(name: String, age: u32) -> Result<Self, ValidationError> {
        if age > 150 {
            return Err(ValidationError::InvalidAge(age));
        }
        Ok(Self { name, age })
    }

    pub fn name(&self) -> &str { &self.name }
    pub fn age(&self) -> u32 { self.age }
    // No setters -- enforce invariants through the constructor.
    // If mutation is needed, provide domain-specific methods.
}
```

**Detection:**
```bash
# Find get_/set_ method pairs (Java-style getters/setters)
grep -rn 'fn get_\|fn set_' crates/ --include='*.rs'
```

---

### Pattern 16: Verbose Match Where Combinators Suffice `[AUTO]`

**Why it is slop:** AI writes explicit match arms for `Option` and `Result` when idiomatic Rust
uses combinators (`.map()`, `.and_then()`, `.unwrap_or()`, `?`). The verbose match is not wrong,
but it is 8 lines where 1 would do, and it obscures the intent behind boilerplate.

**Bad:**
```rust
fn get_display_name(user: &User) -> String {
    match &user.display_name {
        Some(name) => name.clone(),
        None => match &user.username {
            Some(uname) => uname.clone(),
            None => "Anonymous".to_string(),
        },
    }
}
```

**Correct:**
```rust
fn get_display_name(user: &User) -> &str {
    user.display_name
        .as_deref()
        .or(user.username.as_deref())
        .unwrap_or("Anonymous")
}
```

**Detection:**
```bash
cargo clippy -- \
    -D clippy::manual_map \
    -D clippy::manual_unwrap_or \
    -D clippy::manual_unwrap_or_default \
    -D clippy::match_like_matches_macro \
    -D clippy::option_if_let_else
```

---

### Pattern 17: Stringly-Typed APIs `[MANUAL]`

**Why it is slop:** AI uses `String` for concepts that should be enums or newtypes. Status
fields become `"active"` / `"inactive"` strings. Roles become `"admin"` / `"user"` strings.
This pushes validation to runtime, enables typos, and loses exhaustive match checking.

**Bad:**
```rust
fn check_permission(role: &str, action: &str) -> bool {
    match role {
        "admin" => true,
        "editor" => action == "read" || action == "write",
        "viewer" => action == "read",
        _ => false, // Typos silently denied
    }
}
```

**Correct:**
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role { Admin, Editor, Viewer }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action { Read, Write, Delete }

fn check_permission(role: Role, action: Action) -> bool {
    match (role, action) {
        (Role::Admin, _) => true,
        (Role::Editor, Action::Read | Action::Write) => true,
        (Role::Viewer, Action::Read) => true,
        _ => false,
    }
    // Compiler enforces exhaustiveness. Typos are compile errors.
}
```

**Detection:**
```bash
# Find match arms on string literals (potential stringly-typed code)
grep -rn 'match.*{' crates/ --include='*.rs' -A5 | grep '"[a-z_]*"'
```

---

### Pattern 18: Using to_string() / to_owned() Where Into Suffices `[AUTO]`

**Why it is slop:** AI scatters `.to_string()` calls everywhere when constructing structs or
passing arguments. Often, the callee accepts `impl Into<String>`, making the explicit conversion
unnecessary. Worse, AI uses `.to_string()` on `&str` when `.to_owned()` is semantically clearer
(and slightly faster for `&str` specifically, since `to_string` goes through the `Display` trait
formatting machinery).

**Bad:**
```rust
let config = Config {
    host: "localhost".to_string(),
    path: "/api".to_string(),
    name: some_str_ref.to_string(),
};
```

**Correct:**
```rust
let config = Config {
    host: "localhost".into(),  // If field is String and From<&str> works
    path: "/api".into(),
    name: some_str_ref.to_owned(), // Semantically clear: &str -> String
};
```

**Detection:**
```bash
cargo clippy -- -D clippy::str_to_string -D clippy::string_to_string
```

---

## Category 5: Documentation Slop

### Pattern 19: Echo Comments (Restating the Code) `[MANUAL]`

**Why it is slop:** AI generates comments on nearly every line that restate what the code already
says. These add visual noise, increase maintenance burden (the comment must be updated when the
code changes), and actively harm readability by making the developer read everything twice. This
is the single most common AI slop pattern (90-100% occurrence per OX Security). Good comments
explain *why*, not *what*.

**Bad:**
```rust
/// Struct representing a user
pub struct User {
    /// The user's name
    pub name: String,
    /// The user's email
    pub email: String,
    /// Whether the user is active
    pub is_active: bool,
}

/// Creates a new user
pub fn create_user(name: String, email: String) -> User {
    // Create the user struct
    let user = User {
        name,    // Set the name
        email,   // Set the email
        is_active: true, // Set active to true
    };
    // Return the user
    user
}
```

**Correct:**
```rust
/// Account holder in the workspace. Created during onboarding
/// and persisted in the session store.
pub struct User {
    pub name: String,
    pub email: String,
    /// New users default to active. Deactivated by admin action
    /// or after 90 days of inactivity (see `deactivation_policy`).
    pub is_active: bool,
}

pub fn create_user(name: String, email: String) -> User {
    User { name, email, is_active: true }
}
```

**Detection:**
```bash
# Find inline comments that parrot the code (heuristic: comments containing
# the same words as the adjacent code)
grep -rn '// ' crates/ --include='*.rs' | grep -iE '(set the|create the|return the|get the|check if|initialize)'
```

---

### Pattern 20: Missing Doc Comments on Public API `[AUTO]`

**Why it is slop:** AI generates public functions, structs, and traits without `///` doc comments
-- or generates them on private items where they are less useful while skipping the public API.
Rust's ecosystem convention is that all public items have doc comments. Missing docs on public
API is a lint failure and makes `cargo doc` output useless.

**Bad:**
```rust
pub enum Decision {
    Allow,
    Ask,
    Deny,
}

pub fn evaluate(effect: &Effect) -> Decision {
    // ...
}
```

**Correct:**
```rust
/// Outcome of a policy evaluation for a requested effect.
///
/// The policy engine returns one of these for every effect a tool declares
/// before execution.
pub enum Decision {
    /// Effect is permitted without user intervention.
    Allow,
    /// Effect requires explicit user approval before proceeding.
    /// Resolution depends on [`Session::approval_mode`].
    Ask,
    /// Effect is forbidden by policy. Execution must not proceed.
    Deny,
}

/// Evaluate a single effect against the active policy configuration.
///
/// Returns [`Decision::Deny`] for any effect not explicitly permitted.
pub fn evaluate(effect: &Effect) -> Decision {
    // ...
}
```

**Detection:**
```bash
# Clippy lint for missing docs
cargo clippy -- -D clippy::missing_docs_in_private_items
# Or the built-in rustc lint (public items only):
# Add #![warn(missing_docs)] to lib.rs
```

---

### Pattern 21: Hallucinated API References in Comments `[MANUAL]`

**Why it is slop:** AI generates doc comments that reference functions, types, or modules that
do not exist in the codebase. These look authoritative but are fabricated from training data.
They mislead anyone who tries to follow the reference. This is the documentation equivalent
of hallucinated package imports.

**Bad:**
```rust
/// Validates input using the `InputValidator` from `grokrs-validation`.
/// See also: `ValidationPipeline::run()` for batch validation.
pub fn validate(input: &str) -> bool {
    // Neither InputValidator nor ValidationPipeline exist anywhere.
    !input.is_empty()
}
```

**Correct:**
```rust
/// Validates that input is non-empty. Returns `false` for empty or
/// whitespace-only strings.
///
/// Used by [`ToolSpec::execute`] before effect classification.
pub fn validate(input: &str) -> bool {
    !input.is_empty()
}
```

**Detection:**
```bash
# Check that all doc-link references resolve
cargo doc --no-deps 2>&1 | grep 'unresolved link'

# Find references to crates/modules that do not exist
grep -rn '`grokrs-' crates/ --include='*.rs' | grep -v 'Cargo.toml'
```

---

## Category 6: Testing Slop

### Pattern 22: Happy-Path-Only Tests `[MANUAL]`

**Why it is slop:** AI generates tests that verify correct input produces correct output. Period.
No tests for empty input, null equivalents, boundary values, error conditions, or concurrent
access. OX Security found "the lie of unit test code coverage" at 40-70% frequency: high
coverage numbers that validate the AI's own assumptions, not the system's actual requirements.

**Bad:**
```rust
#[test]
fn test_parse_config() {
    let toml = r#"
        [workspace]
        root = "/tmp/test"
    "#;
    let config = AppConfig::from_str(toml).unwrap();
    assert_eq!(config.workspace.root, "/tmp/test");
}
```

**Correct:**
```rust
#[test]
fn parses_valid_config() {
    let toml = r#"
        [workspace]
        root = "/tmp/test"
    "#;
    let config = AppConfig::from_str(toml).expect("valid TOML should parse");
    assert_eq!(config.workspace.root, "/tmp/test");
}

#[test]
fn rejects_empty_config() {
    let result = AppConfig::from_str("");
    assert!(result.is_err(), "empty input must fail");
}

#[test]
fn rejects_missing_workspace_root() {
    let toml = r#"
        [workspace]
    "#;
    let err = AppConfig::from_str(toml).unwrap_err();
    assert!(format!("{err}").contains("root"), "error should mention missing field");
}

#[test]
fn rejects_relative_workspace_root() {
    let toml = r#"
        [workspace]
        root = "relative/path"
    "#;
    let err = AppConfig::from_str(toml).unwrap_err();
    assert!(format!("{err}").contains("absolute"), "error should require absolute path");
}

#[test]
fn handles_extra_unknown_fields_gracefully() {
    let toml = r#"
        [workspace]
        root = "/tmp/test"
        unknown_field = "value"
    "#;
    // Decide: should this warn or fail? Test the chosen behavior.
    let result = AppConfig::from_str(toml);
    assert!(result.is_ok(), "unknown fields should be ignored (serde default)");
}
```

**Detection:**
```bash
# Count test functions vs assertion variety
grep -rn '#\[test\]' crates/ --include='*.rs' | wc -l
grep -rn 'assert.*is_err\|assert.*is_none\|should_panic' crates/ --include='*.rs' | wc -l
# If the second number is much smaller than the first, error paths are undertested.
```

---

### Pattern 23: Tests That Test the Mock, Not the Code `[MANUAL]`

**Why it is slop:** AI generates test doubles that are so tightly coupled to the implementation
that the test only verifies the mock returns what it was told to return. The actual code under
test is barely exercised. This gives the illusion of coverage while testing nothing real.

**Bad:**
```rust
#[test]
fn test_fetch_user() {
    let mut mock_db = MockDatabase::new();
    mock_db.expect_get_user()
        .with(eq(42))
        .returning(|_| Ok(User { id: 42, name: "Alice".into() }));

    let result = fetch_user(&mock_db, 42).unwrap();
    assert_eq!(result.name, "Alice"); // Only proves the mock returns what we told it to
}
```

**Correct:**
```rust
#[test]
fn fetch_user_returns_none_for_missing_id() {
    let db = TestDatabase::in_memory();
    // No user inserted -- database is empty
    let result = fetch_user(&db, 42);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), StoreError::NotFound { .. }));
}

#[test]
fn fetch_user_returns_inserted_user() {
    let db = TestDatabase::in_memory();
    db.insert_user(&User { id: 42, name: "Alice".into() }).unwrap();

    let user = fetch_user(&db, 42).unwrap();
    assert_eq!(user.id, 42);
    assert_eq!(user.name, "Alice");
}
```

**Detection:**
```bash
# Find mock-heavy tests (heuristic: lots of expect/returning calls)
grep -rn 'expect_\|\.returning(' crates/ --include='*.rs' | wc -l
# Compare to the number of real assertions
grep -rn 'assert' crates/ --include='*.rs' | grep -v 'debug_assert' | wc -l
```

---

### Pattern 24: Redundant Test Cases (Testing Masturbation) `[MANUAL]`

**Why it is slop:** AI generates many test cases that are semantically identical with trivially
different inputs. Five tests that all verify "valid input returns Ok" with different valid inputs
add maintenance cost without adding confidence. Meanwhile, the error paths, boundaries, and
concurrency scenarios go untested. As one engineering lead put it: "That is not exhaustiveness,
it is throwing things at the wall."

**Bad:**
```rust
#[test] fn parses_port_80()    { assert!(parse_port("80").is_ok()); }
#[test] fn parses_port_443()   { assert!(parse_port("443").is_ok()); }
#[test] fn parses_port_8080()  { assert!(parse_port("8080").is_ok()); }
#[test] fn parses_port_3000()  { assert!(parse_port("3000").is_ok()); }
#[test] fn parses_port_9090()  { assert!(parse_port("9090").is_ok()); }
// Five tests, one equivalence class. Zero boundary tests.
```

**Correct:**
```rust
#[test]
fn parses_valid_port() {
    assert_eq!(parse_port("80").unwrap(), 80);
    assert_eq!(parse_port("443").unwrap(), 443);
}

#[test]
fn rejects_zero_port() {
    assert!(parse_port("0").is_err());
}

#[test]
fn rejects_port_above_65535() {
    assert!(parse_port("65536").is_err());
}

#[test]
fn rejects_non_numeric() {
    assert!(parse_port("abc").is_err());
    assert!(parse_port("").is_err());
    assert!(parse_port("-1").is_err());
}

#[test]
fn boundary_ports() {
    assert_eq!(parse_port("1").unwrap(), 1);
    assert_eq!(parse_port("65535").unwrap(), 65535);
}
```

**Detection:**
```bash
# Find test modules with many similarly-named tests
grep -rn '#\[test\]' crates/ --include='*.rs' -A1 | grep 'fn test_' | \
    sed 's/[0-9]//g' | sort | uniq -c | sort -rn | head -20
# High counts with similar names suggest redundant test cases.
```

---

### Pattern 25: No Property-Based or Fuzz Testing `[MANUAL]`

**Why it is slop:** AI generates only example-based tests. It never uses property-based testing
(`proptest`, `quickcheck`) or fuzzing to explore the input space. For a safety-focused codebase
like `grokrs` where paths must not escape workspaces and policy must deny by default, property
tests are essential to verify invariants hold across all inputs, not just the handful the AI
happened to think of.

**Bad:** Entire test suite consists of `#[test]` functions with hardcoded inputs.

**Correct:**
```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn workspace_path_never_escapes_root(
        input in "([a-zA-Z0-9._/-]{0,200})"
    ) {
        if let Ok(path) = WorkspacePath::new(&input) {
            // Invariant: resolved path must not contain ..
            let resolved = path.as_str();
            assert!(!resolved.contains(".."),
                "WorkspacePath must reject traversal: got {resolved}");
        }
        // Err is fine -- rejection is correct behavior
    }

    #[test]
    fn policy_denies_unknown_effects(
        domain in "[a-z]{1,50}\\.[a-z]{2,5}"
    ) {
        let engine = PolicyEngine::deny_by_default();
        let effect = Effect::NetworkConnect { host: domain };
        let decision = engine.evaluate(&effect);
        assert_eq!(decision, Decision::Deny,
            "deny-by-default must deny unknown network targets");
    }
}
```

**Detection:**
```bash
# Check if proptest/quickcheck is in use at all
grep -rn 'proptest\|quickcheck' crates/ --include='*.rs' | wc -l
grep -rn 'proptest\|quickcheck' Cargo.toml crates/*/Cargo.toml | wc -l
# Zero means the test suite has no randomized coverage.
```

---

## Compound Checklist for Pull Request Review

Use this condensed checklist during PR review. Each item maps to one or more patterns above.

### Safety
- [ ] No `.unwrap()` or `.expect()` in production paths (P1, P2)
- [ ] No `unsafe` without `// SAFETY:` justification (P3)
- [ ] All `Result` and `Option` error paths handled, not swallowed (P4)
- [ ] No bare index access on slices/vecs (P5)

### Performance
- [ ] No unnecessary `.clone()` -- borrowing verified as insufficient (P6)
- [ ] Function parameters use `&str` not `String` where possible (P7)
- [ ] No intermediate `.collect()` that is immediately iterated (P8)
- [ ] No allocations inside tight loops that could be hoisted (P9)
- [ ] No `format!()` in comparisons or hash key construction (P10)

### Structure
- [ ] No function over 50 lines (P11)
- [ ] No reimplementation of existing shared code (P12)
- [ ] No trait/abstraction with only one implementation (P13)
- [ ] No variant/residue files (P14)

### Idioms
- [ ] No Java-style getter/setter pairs (P15)
- [ ] No verbose match where a combinator would work (P16)
- [ ] No stringly-typed enums (P17)
- [ ] No gratuitous `.to_string()` where `.into()` or `.to_owned()` fits (P18)

### Documentation
- [ ] No comments that restate the code (P19)
- [ ] All public items have meaningful doc comments (P20)
- [ ] No references to non-existent types or modules in docs (P21)

### Testing
- [ ] Error paths and boundary conditions are tested (P22)
- [ ] Tests exercise real code, not mocks returning canned data (P23)
- [ ] No clusters of semantically identical test cases (P24)
- [ ] Property-based tests cover key invariants (P25)

---

## Automated Enforcement

### Clippy Configuration

Add to the workspace `clippy.toml`:
```toml
too-many-lines-threshold = 50
```

Add to CI or `Cargo.toml` (via `[lints]`):
```toml
[lints.clippy]
unwrap_used = "deny"
expect_used = "warn"
clone_on_copy = "deny"
redundant_clone = "deny"
needless_collect = "deny"
ptr_arg = "deny"
manual_map = "warn"
manual_unwrap_or = "warn"
indexing_slicing = "warn"
too_many_lines = "warn"
missing_docs_in_private_items = "allow"
str_to_string = "warn"
```

Add to each crate's `lib.rs`:
```rust
#![warn(missing_docs)]
```

### CI Script

```bash
#!/usr/bin/env bash
set -euo pipefail

echo "=== AI Slop Detection ==="

echo "--- Checking for unwrap/expect in production code ---"
UNWRAP_COUNT=$(grep -rn '\.unwrap()' crates/ --include='*.rs' | grep -cv '/tests/\|#\[cfg(test)\]\|#\[test\]' || true)
echo "Found $UNWRAP_COUNT unwrap() calls outside tests"

echo "--- Checking for unnecessary clones ---"
cargo clippy -- -D clippy::redundant_clone 2>&1 | tail -5

echo "--- Checking for unsafe without SAFETY comment ---"
UNSAFE_COUNT=$(grep -rn 'unsafe {' crates/ --include='*.rs' | wc -l)
SAFETY_COUNT=$(grep -rn '// SAFETY:' crates/ --include='*.rs' | wc -l)
echo "unsafe blocks: $UNSAFE_COUNT, SAFETY comments: $SAFETY_COUNT"
if [ "$UNSAFE_COUNT" -gt "$SAFETY_COUNT" ]; then
    echo "WARNING: Some unsafe blocks lack SAFETY comments"
fi

echo "--- Checking for debugging residue files ---"
find crates/ -name '*_old.rs' -o -name '*_v2.rs' -o -name '*_backup.rs' \
    -o -name '*_temp.rs' -o -name '*_fixed.rs' -o -name '*_wip.rs'

echo "--- Checking for Java-style getters/setters ---"
grep -rn 'fn get_\|fn set_' crates/ --include='*.rs' | grep -v '/tests/' || true

echo "--- Checking doc coverage ---"
cargo doc --no-deps 2>&1 | grep -c 'missing documentation' || echo "0 missing docs warnings"

echo "=== Done ==="
```

---

## Sources

### Primary Research
- **CodeRabbit** (2025): Analysis of 470 open-source GitHub PRs. AI-co-authored PRs contain 10.83 issues per PR vs 6.45 for human-only. Logic errors 75% more common, security vulnerabilities 2.74x rate.
- **GitClear** (2025): 211 million changed lines analyzed. Copy/paste code rose from 8.3% to 12.3%, refactoring dropped from 25% to under 10%.
- **OX Security** (2025): 300 AI-generated repos vs 250 human-coded baselines. 10 distinct anti-pattern categories identified.
- **Snyk** (2025): 36-40% of AI-generated code snippets contain security vulnerabilities.
- **CMU** (2025): 807 repos studied. Static analysis warnings rose ~30% post-AI-adoption; code complexity rose 40%+.
- **ACM** (2024): 29.5% of Python, 24.2% of JavaScript Copilot-generated snippets contain security weaknesses across 43 CWE categories.
- **Apiiro** (2025): 10x increase in security findings per month at Fortune 50 enterprises (1,000 to 10,000+).

### Articles and Analysis
- Aviator: "How to Avoid AI Code Slop" (2026-03-17) -- Taxonomy of AI slop, intent-driven development
- localskills.sh: "AI Coding Rules for Rust Projects" (2026-02-19) -- Rust-specific AI rules
- Luis Garcia: "Making AI-Generated Rust Code Trustworthy" (2026-01-05) -- Layered defense model with Clippy and Dylint
- Matt Basta: "The Imminent Risk of Vibe Coding" (2026-01-30) -- Negative feedback loops from AI-generated PRs
- Christopher Montes: "Lint Against the Machine" (2026-03-06) -- Field guide to AI coding agent anti-patterns, detection with pre-commit hooks
- Agent Patterns: "Pattern Replication Risk" -- Agents reproduce deprecated patterns at scale
- SoftwareSeni: "Understanding Anti-Patterns and Quality Degradation in AI-Generated Code" (2025-12-10) -- OX Security research deep dive
- Propel Code: "Emergent Code Review Patterns for AI-Generated Code" (2025-08-28) -- Three-layer review framework
- TechDebt.guru: "AI Code Review Guide & Checklist" (2026-02-24) -- The "looks right" trap, red flags
- Sesame Disk: "Troubleshooting LLM-Generated Code" (2026-03-08) -- Failure patterns in LLM code
- Dr. Derek Austin: "LLMs Have Revived These 5 Anti-Patterns" (2026-02-06) -- Over-commenting, helper function proliferation
- reintech.io: "10 Common Rust Mistakes" (2026-03-19) -- Ownership, clone abuse, unwrap misuse

### Academic Papers
- Wang et al., "Towards Understanding the Characteristics of Code Generation Errors Made by Large Language Models" (ICSE 2025)
- Chen et al., "A Deep Dive Into Large Language Model Code Generation Mistakes: What and Why?" (arXiv 2411.01414, 2024)
- Song et al., "An Empirical Study of Code Generation Errors made by Large Language Models" (MAPS 2023)
