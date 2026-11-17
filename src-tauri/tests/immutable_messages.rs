use std::fs;
use std::path::{Path, PathBuf};

struct Statement {
    file: String,
    line: usize,
    sql: String,
}

fn store_files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            store_files(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && path.file_name().is_some_and(|name| name != "migrations.rs")
        {
            found.push(path);
        }
    }
}

fn literals(source: &str) -> Vec<Vec<(usize, char)>> {
    let chars: Vec<char> = source.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    let mut found = Vec::new();
    let mut line = 1;
    let mut i = 0;
    while let Some(c) = at(i) {
        let after_word = i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c == '/' && at(i + 1) == Some('/') {
            while at(i).is_some_and(|c| c != '\n') {
                i += 1;
            }
        } else if c == '/' && at(i + 1) == Some('*') {
            i += 2;
            while at(i).is_some() && !(at(i) == Some('*') && at(i + 1) == Some('/')) {
                line += usize::from(chars[i] == '\n');
                i += 1;
            }
            i += 2;
        } else if c == '\'' {
            i += if at(i + 1) == Some('\\') {
                2 + chars[i + 2..].iter().position(|&c| c == '\'').map_or(0, |end| end + 1)
            } else if at(i + 2) == Some('\'') {
                3
            } else {
                1
            };
        } else if c == 'r' && !after_word && matches!(at(i + 1), Some('"' | '#')) {
            let hashes = chars[i + 1..].iter().take_while(|&&c| c == '#').count();
            if at(i + 1 + hashes) != Some('"') {
                i += 1;
                continue;
            }
            i += 2 + hashes;
            let mut text = Vec::new();
            while let Some(c) = at(i) {
                if c == '"'
                    && chars[i + 1..].iter().take(hashes).filter(|&&c| c == '#').count() == hashes
                {
                    i += 1 + hashes;
                    break;
                }
                text.push((line, c));
                line += usize::from(c == '\n');
                i += 1;
            }
            found.push(text);
        } else if c == '"' {
            i += 1;
            let mut text = Vec::new();
            while let Some(c) = at(i) {
                match c {
                    '"' => {
                        i += 1;
                        break;
                    }
                    '\\' if at(i + 1) == Some('\n') => {
                        i += 1;
                        while at(i).is_some_and(char::is_whitespace) {
                            line += usize::from(chars[i] == '\n');
                            i += 1;
                        }
                    }
                    '\\' => {
                        let escaped = at(i + 1).unwrap_or(' ');
                        let decoded =
                            if matches!(escaped, 'n' | 't' | 'r') { ' ' } else { escaped };
                        text.push((line, decoded));
                        i += 2;
                    }
                    _ => {
                        text.push((line, c));
                        line += usize::from(c == '\n');
                        i += 1;
                    }
                }
            }
            found.push(text);
        } else {
            i += 1;
        }
    }
    found
}

fn statements(file: &str, source: &str) -> Vec<Statement> {
    literals(source)
        .iter()
        .flat_map(|text| text.split(|&(_, c)| c == ';'))
        .filter_map(|piece| {
            let start = piece.iter().position(|&(_, c)| !c.is_whitespace())?;
            let sql: String = piece.iter().map(|&(_, c)| c).collect();
            Some(Statement {
                file: file.to_owned(),
                line: piece[start].0,
                sql: sql.split_whitespace().collect::<Vec<_>>().join(" "),
            })
        })
        .collect()
}

fn words(sql: &str) -> Vec<String> {
    sql.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | ','))
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn names_messages(word: Option<&String>) -> bool {
    word.is_some_and(|word| {
        let name = word.rsplit('.').next().unwrap_or(word);
        name.trim_matches(|c| matches!(c, '"' | '`' | '[' | ']')) == "messages"
    })
}

fn rewrites_messages(sql: &str) -> bool {
    let words = words(sql);
    let upserts = words.windows(2).any(|pair| pair == ["do", "update"]);
    words.iter().enumerate().any(|(at, word)| match word.as_str() {
        "update" if at == 0 || words[at - 1] != "do" => {
            let target =
                if words.get(at + 1).is_some_and(|next| next == "or") { at + 3 } else { at + 1 };
            names_messages(words.get(target))
        }
        "replace" => {
            words.get(at + 1).is_some_and(|next| next == "into")
                && names_messages(words.get(at + 2))
        }
        "into" => upserts && names_messages(words.get(at + 1)),
        _ => false,
    })
}

fn begins(sql: &str, prefix: &[&str]) -> bool {
    let words = words(sql);
    words.len() >= prefix.len() && words.iter().zip(prefix).all(|(word, want)| word == want)
}

fn scan(dir: &Path) -> Vec<Statement> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    store_files(dir, &mut files);
    files.sort();
    files
        .iter()
        .flat_map(|path| {
            let shown = path.strip_prefix(root).unwrap_or(path).display().to_string();
            statements(&shown, &fs::read_to_string(path).unwrap())
        })
        .collect()
}

fn store_statements() -> Vec<Statement> {
    scan(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src/db"))
}

#[test]
fn no_statement_in_the_store_updates_a_saved_message() {
    let rewrites: Vec<String> = store_statements()
        .into_iter()
        .filter(|statement| rewrites_messages(&statement.sql))
        .map(|statement| format!("{}:{}: {}", statement.file, statement.line, statement.sql))
        .collect();
    assert!(
        rewrites.is_empty(),
        "A saved message is never changed, only inserted or deleted, but these statements rewrite one:\n{}",
        rewrites.join("\n")
    );
}

#[test]
fn the_scan_reads_the_store_inserts_and_deletes_of_messages() {
    let found = store_statements();
    let store = |prefix: &[&str]| {
        found.iter().filter(|s| s.file.ends_with("db/store.rs") && begins(&s.sql, prefix)).count()
    };
    assert!(store(&["insert", "into", "messages"]) >= 1);
    assert!(store(&["delete", "from", "messages"]) >= 2);
    assert!(found.iter().any(|s| s.sql == "DELETE FROM messages WHERE channel_id=?1"));
    assert!(found.iter().all(|s| !s.file.ends_with("migrations.rs")));
}

#[test]
fn the_scan_finds_the_update_that_a_shipped_migration_made() {
    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/db/migrations.rs"))
            .unwrap();
    let found = statements("src/db/migrations.rs", &source);
    assert!(found
        .iter()
        .any(|s| rewrites_messages(&s.sql) && s.sql.contains(") UPDATE messages SET parts = (")));
}

#[test]
fn every_form_of_rewriting_a_message_is_caught_whatever_its_case_and_spacing() {
    for sql in [
        "UPDATE messages SET parts=?2 WHERE id=?1",
        "update\n   messages set hop=0",
        "UPDATE OR IGNORE messages SET hop=0",
        "update or replace \"messages\" set hop=0",
        "UPDATE main.messages SET hop=0",
        "INSERT OR REPLACE INTO messages (id) VALUES (?1)",
        "insert   or replace into messages(id) values(?1)",
        "REPLACE INTO messages (id) VALUES (?1)",
        "replace into `messages` (id) values (?1)",
        "INSERT INTO messages (id,hop) VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET hop=excluded.hop",
        "insert into messages(id) values(?1) on conflict do\n update set hop=0",
        "WITH x AS (SELECT 1) UPDATE messages SET hop=0",
    ] {
        assert!(rewrites_messages(sql), "{sql}");
    }
    for sql in [
        "INSERT INTO messages (id) VALUES (?1)",
        "INSERT INTO messages (id) VALUES (?1) ON CONFLICT(id) DO NOTHING",
        "DELETE FROM messages WHERE channel_id=?1",
        "UPDATE pending_runs SET started_at=?2 WHERE run_id=(SELECT run_id FROM messages WHERE id=?1)",
        "INSERT INTO outbox (id) SELECT id FROM messages ON CONFLICT(id) DO UPDATE SET id=excluded.id",
        "UPDATE messages_seen SET at=?1",
        "SELECT replace(parts, 'a', 'b') FROM messages",
    ] {
        assert!(!rewrites_messages(sql), "{sql}");
    }
}

#[test]
fn statements_carry_the_line_they_start_on_through_escapes_and_raw_strings() {
    let source = "fn f() {\n    let c = '\"';\n    x(\"SELECT 1;\n  update messages \\\n  set hop=0\");\n    y(r#\"\nREPLACE INTO messages (id) VALUES (\"a\")\"#);\n}\n";
    let found = statements("f.rs", source);
    let shown: Vec<(usize, &str)> = found.iter().map(|s| (s.line, s.sql.as_str())).collect();
    assert_eq!(
        shown,
        [
            (3, "SELECT 1"),
            (4, "update messages set hop=0"),
            (7, "REPLACE INTO messages (id) VALUES (\"a\")"),
        ]
    );
}
