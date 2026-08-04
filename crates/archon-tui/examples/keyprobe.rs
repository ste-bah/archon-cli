//! Print the raw `KeyEvent` for every keypress, then exit on Ctrl+C.
//!
//! Diagnostic only. The TUI keymap is a `HashMap<KeyEvent, Action>`, and
//! `KeyEvent` hashes over ALL of its fields -- `code`, `modifiers`, `kind` AND
//! `state`. A binding built with `KeyEvent::new(code, modifiers)` carries
//! `kind: Press` and `state: NONE`, so an otherwise-identical event with a
//! different `kind` or `state` silently fails to resolve. That is invisible in
//! the source, so this prints what the terminal actually delivers.
//!
//! Run:  cargo run -p archon-tui --example keyprobe

use crossterm::event::{Event, KeyCode, KeyModifiers, read};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

fn main() -> std::io::Result<()> {
    enable_raw_mode()?;
    println!("Press keys. Ctrl+C exits.\r");
    println!("Try: ? @ ~ {{ }} \" and a plain letter for comparison.\r");
    println!("---\r");

    loop {
        if let Event::Key(key) = read()? {
            println!(
                "code={:?}  modifiers={:?}  kind={:?}  state={:?}\r",
                key.code, key.modifiers, key.kind, key.state
            );
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                break;
            }
        }
    }

    disable_raw_mode()?;
    Ok(())
}
