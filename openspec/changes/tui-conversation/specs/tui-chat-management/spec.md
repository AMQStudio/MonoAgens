# Specification — `tui-chat-management`

## ADDED Requirements

### Requirement: Chats Always Get a Title
After the first message of an unlocked chat the TUI SHALL set a title from the model when it answers within `output_limits.classify` and a 30 s timeout, otherwise from a deterministic keyphrase of the first message, and SHALL log failures under `titler`.

#### Scenario: Provider fails to title
- **WHEN** the first message is "tolong jelaskan cara kerja valve rbac" and the provider returns an error for the title request
- **THEN** the title becomes a keyphrase of at most 40 characters from that message instead of "New Chat", and the log has a `titler` warning with the error.

### Requirement: Topic-Shift Re-Titling Uses the Embedder When Present
Topic-shift detection SHALL use cosine similarity against the chat's stored centroid when `[embed]` is configured and the existing n-gram rule otherwise, and a manual rename SHALL keep the title locked.

#### Scenario: Locked title
- **WHEN** a chat was renamed with `/chat rename` and the topic then shifts
- **THEN** the title does not change.

### Requirement: Chat Search
The user SHALL be able to filter chats by title and message text.

#### Scenario: Search by message text
- **WHEN** the user searches for a word that occurs only in one chat's messages
- **THEN** only that chat is listed.

### Requirement: Pinned Chats
`/chat pin` SHALL toggle a chat's pinned flag, and pinned chats SHALL sort first in the sidebar.

#### Scenario: Pin sorts first
- **WHEN** an older chat is pinned
- **THEN** it appears above newer unpinned chats and the flag persists in `tui_chats.json`.

### Requirement: Delete Asks for Confirmation
Deleting a chat SHALL ask for confirmation and SHALL leave the chat unchanged when declined.

#### Scenario: Decline
- **WHEN** the user runs delete and answers `n`
- **THEN** the chat remains.

### Requirement: Export to Markdown
`/export` SHALL write the current chat as Markdown containing message text and attachment filenames and no secrets, to the given path or to `<data_dir>/exports/`.

#### Scenario: Export file
- **WHEN** the user runs `/export` on a chat with two messages
- **THEN** a Markdown file exists with both messages in order and a toast shows its path.
