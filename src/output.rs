//! What goes to stdout (the service's answers) and to stderr (everything said to the person). No colour, no
//! spinner: the output is the same whether or not it is a terminal.

use std::io::{IsTerminal, Write};

use anyhow::{bail, Result};
use serde::Serialize;
use serde_json::Value;

/// Says something to the person, on stderr.
#[macro_export]
macro_rules! say {
    ($($arg:tt)*) => { eprintln!($($arg)*) };
}

/// Pretty JSON on stdout.
pub fn json<T: Serialize + ?Sized>(value: &T) -> Result<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, value)?;
    writeln!(out)?;
    Ok(())
}

/// Aligned columns on stdout; the last column is not padded.
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> Result<()> {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let mut out = std::io::stdout().lock();
    let mut line = |cells: Vec<&str>| -> Result<()> {
        let last = cells.len().saturating_sub(1);
        let mut text = String::new();
        for (i, (cell, w)) in cells.iter().zip(&widths).enumerate() {
            if i == last {
                text.push_str(cell);
            } else {
                text.push_str(&format!("{cell:<w$}  "));
            }
        }
        writeln!(out, "{}", text.trim_end())?;
        Ok(())
    };
    line(headers.to_vec())?;
    for row in rows {
        line(row.iter().map(String::as_str).collect())?;
    }
    Ok(())
}

/// The first line of a description, cut to fit a column.
pub fn summary(text: Option<&str>, max: usize) -> String {
    let line = text.unwrap_or("").lines().next().unwrap_or("").trim();
    cut(line, max)
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// A cell of a table made from JSON: text as it is, numbers and booleans as text, nothing for null, anything
/// nested as one line of JSON.
pub fn cell(value: &Value) -> String {
    let text = match value {
        Value::Null => String::new(),
        Value::String(s) => s.replace(['\n', '\r', '\t'], " "),
        other => other.to_string(),
    };
    cut(&text, 60)
}

/// A query's answer, as a person reads it: a list of objects is a table (the answer itself, or its one list
/// member, with the other members after it); anything else is JSON.
pub fn answer(value: &Value) -> Result<()> {
    if let Some((list, rest)) = rows_of(value) {
        let mut headers: Vec<String> = Vec::new();
        for item in &list {
            for key in item.as_object().into_iter().flat_map(|o| o.keys()) {
                if !headers.contains(key) {
                    headers.push(key.clone());
                }
            }
        }
        let rows: Vec<Vec<String>> = list
            .iter()
            .map(|item| headers.iter().map(|h| cell(&item[h.as_str()])).collect())
            .collect();
        table(&headers.iter().map(String::as_str).collect::<Vec<_>>(), &rows)?;
        for (k, v) in rest {
            println!("{k}: {}", cell(&v));
        }
        return Ok(());
    }
    match value {
        Value::String(s) => {
            println!("{s}");
            Ok(())
        }
        other => json(other),
    }
}

/// The list to show as a table, and the other members of the answer.
type Rows = (Vec<Value>, Vec<(String, Value)>);

fn rows_of(value: &Value) -> Option<Rows> {
    let is_list = |v: &Value| {
        v.as_array()
            .is_some_and(|a| !a.is_empty() && a.iter().all(Value::is_object))
    };
    match value {
        Value::Array(_) if is_list(value) => Some((value.as_array().cloned().unwrap_or_default(), Vec::new())),
        Value::Object(object) => {
            let lists: Vec<&String> = object.iter().filter(|(_, v)| is_list(v)).map(|(k, _)| k).collect();
            let [only] = lists.as_slice() else { return None };
            let list = object[only.as_str()].as_array().cloned().unwrap_or_default();
            let rest = object
                .iter()
                .filter(|(k, _)| k != only)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            Some((list, rest))
        }
        _ => None,
    }
}

/// Asks before a high-impact command. `yes` answers for a script; with no terminal to ask on, the command is not
/// sent.
pub fn confirm(question: &str, yes: bool) -> Result<()> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        bail!(NotConfirmed(format!(
            "{question} — not asked: there is no terminal. Pass --yes to send it."
        )));
    }
    eprint!("{question} [y/N] ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    if matches!(line.trim(), "y" | "Y" | "yes" | "Yes") {
        Ok(())
    } else {
        bail!(NotConfirmed("not sent".into()))
    }
}

/// The person (or the lack of one) did not confirm. Exits with its own code.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct NotConfirmed(pub String);

/// The command line asked for something that cannot be done as written. Exits 2, like a parse error.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Usage(pub String);

/// There is no connection to act on. Exits with its own code.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct NotSignedIn(pub String);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_list_is_found_in_an_answer() {
        let (list, rest) = rows_of(&json!({"count": 2, "items": [{"a": 1}, {"a": 2}], "nextCursor": "c"})).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(rest.len(), 2);
        assert!(
            rows_of(&json!({"a": [{"x": 1}], "b": [{"y": 2}]})).is_none(),
            "two lists: JSON"
        );
        assert!(rows_of(&json!({"items": []})).is_none(), "an empty list: JSON");
        assert!(rows_of(&json!([1, 2])).is_none());
        assert_eq!(rows_of(&json!([{"a": 1}])).unwrap().0.len(), 1);
    }

    #[test]
    fn cells_are_one_line_and_bounded() {
        assert_eq!(cell(&json!(null)), "");
        assert_eq!(cell(&json!("a\nb")), "a b");
        assert_eq!(cell(&json!({"k": [1, 2]})), r#"{"k":[1,2]}"#);
        assert_eq!(cell(&json!(12.5)), "12.5");
        assert_eq!(cell(&json!("x".repeat(100))).chars().count(), 60);
        assert_eq!(cut("GUIDED SEQUENCE — part of", 17), "GUIDED SEQUENCE …");
        assert_eq!(cut("héllo", 5), "héllo");
    }

    #[test]
    fn summaries_take_the_first_line() {
        assert_eq!(summary(Some("One.\nTwo."), 80), "One.");
        assert_eq!(summary(None, 80), "");
    }
}
