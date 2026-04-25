//! Synthetic fixture: a single function whose cognitive complexity exceeds 10.
//! The function uses nested `if / else if / match` as expression values
//! (no `return`), so clippy's cognitive-complexity scorer is not reset at
//! each branch and the total exceeds the configured threshold.

#![allow(dead_code)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_else_if)]
#![allow(clippy::needless_return)]
#![allow(clippy::if_same_then_else)]

pub fn classify(n: i32, flag: bool, mode: u8) -> &'static str {
    let a = if n < 0 {
        if flag {
            if n < -100 { "neg-huge-flag" }
            else if n < -10 { "neg-med-flag" }
            else { "neg-small-flag" }
        } else {
            if n < -100 { "neg-huge" }
            else if n < -10 { "neg-med" }
            else { "neg-small" }
        }
    } else if n == 0 {
        if flag { "zero-flag" } else { "zero" }
    } else {
        match mode {
            0 => {
                if n > 1000 { "pos-huge-m0" }
                else if n > 100 { "pos-big-m0" }
                else if n > 10 { "pos-med-m0" }
                else { "pos-small-m0" }
            }
            1 => {
                if n > 1000 { "pos-huge-m1" }
                else if n > 100 { "pos-big-m1" }
                else { "pos-small-m1" }
            }
            2 => {
                if flag { "pos-m2-flag" } else { "pos-m2" }
            }
            _ => {
                if flag { "pos-other-flag" } else { "pos-other" }
            }
        }
    };

    // Second branchy block to push cognitive complexity above threshold.
    let b = if n < 0 {
        if flag {
            if mode == 0 { "x0" }
            else if mode == 1 { "x1" }
            else if mode == 2 { "x2" }
            else { "x?" }
        } else {
            if mode == 0 { "y0" }
            else if mode == 1 { "y1" }
            else { "y?" }
        }
    } else {
        if flag { "pos-flag2" } else { "pos2" }
    };

    if a.len() > b.len() { a } else { b }
}
