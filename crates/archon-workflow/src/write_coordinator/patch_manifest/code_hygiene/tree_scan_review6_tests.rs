//! A sixth review's cases: ordinary edits that flip a callback between
//! container and absorber, or rename its receiver, must not make an
//! untouched over-cap callback read as new or grown.

use super::*;

const MAX: u32 = 15;

fn accepts(path: &str, baseline: &str, post: &str) {
    if let Err(err) = validate_complexity(path, Some(baseline), post, MAX) {
        panic!("{path}: refused: {err}");
    }
}

fn grows(path: &str, baseline: &str, post: &str, expected: &str) {
    match validate_complexity(path, Some(baseline), post, MAX) {
        Ok(notes) => panic!("{path}: accepted ({notes:?})"),
        Err(err) => assert!(err.to_string().contains(expected), "{path}: {err}"),
    }
}

/// Sixteen branch points, `if (aN && bN) { x(); }` eight times.
fn eight(indent: &str) -> String {
    (1..=8)
        .map(|n| format!("{indent}if (a{n} && b{n}) {{ x(); }}\n"))
        .collect()
}

fn express(receiver: &str, before: &str, bind: &str, after: &str) -> String {
    format!(
        "const express = require('express');\nconst app = express();\n\n\
         app.post('/orders', async (req, res) => {{\n  const order = req.body;{before}\n  \
         {bind}await {receiver}.transaction(async (trx) => {{\n{}  }});\n{after}  res.json({{ ok: true }});\n}});\n",
        eight("      ")
    )
}

#[test]
fn edits_that_flip_a_container_leave_an_untouched_callback_alone() {
    let base = express("db", "", "", "");
    accepts(
        "src/o.js",
        &base,
        &express(
            "db",
            "",
            "",
            "  order.items.forEach((item) => audit.log(item));\n",
        ),
    );
    accepts(
        "src/o.js",
        &base,
        &express("db", "", "", "  audit.logAll(order.items);\n"),
    );
    accepts(
        "src/o.js",
        &base,
        &express(
            "db",
            "",
            "const orderId = ",
            "  order.items.forEach((item) => audit.log(item));\n",
        ),
    );
    let jquery = |extra: &str| {
        format!(
            "$(function () {{\n  $('#save').on('click', function () {{\n{}  }});\n{extra}}});\n",
            eight("    ")
        )
    };
    accepts(
        "src/ui.js",
        &jquery(""),
        &jquery("  $('#cancel').on('click', function () {\n    hide();\n  });\n"),
    );
    let ginkgo = |extra: &str| {
        format!(
            "package store_test\n\nvar _ = Describe(\"Store\", func() {{\n\tContext(\"when empty\", func() {{\n\t\tIt(\"reconciles\", func() {{\n{}\t\t}})\n{extra}\t}})\n}})\n",
            eight("\t\t\t")
                .replace("if (a", "if a")
                .replace(") {", " {")
                .replace("x();", "x()")
        )
    };
    accepts(
        "store_test.go",
        &ginkgo(""),
        &ginkgo("\t\tIt(\"has no items\", func() {\n\t\t\tExpect(s.Len()).To(Equal(0))\n\t\t})\n"),
    );
    let rake = |extra: &str| {
        let body: String = (1..=8)
            .map(|n| format!("      if a{n} && b{n} then x end\n"))
            .collect();
        format!(
            "namespace :data do\n  desc \"Backfill\"\n  task backfill: :environment do\n{body}  end\n{extra}end\n"
        )
    };
    accepts(
        "lib/tasks/data.rake",
        &rake(""),
        &rake("\n  desc \"Count\"\n  task count: :environment do\n    puts Record.count\n  end\n"),
    );
}

#[test]
fn renaming_a_receiver_does_not_make_its_callback_new() {
    let base = express("db", "\n  audit.log(order);", "", "");
    let renamed = |after: &str| express("database", " const database = db;", "", after);
    accepts("src/o.js", &express("db", "", "", ""), &renamed(""));
    accepts(
        "src/o.js",
        &express("db", "", "", "  audit.log(order);\n"),
        &renamed("  order.items.forEach((item) => audit.log(item));\n"),
    );
    let _ = base;
    let top = |receiver: &str| {
        format!(
            "const {receiver} = load();\n{receiver}.forEach((row) => {{\n{}}});\n",
            eight("      ")
        )
    };
    accepts("src/t.js", &top("rows"), &top("records"));
}

#[test]
fn a_branch_added_to_an_over_cap_function_is_still_refused() {
    let base = express("db", "", "", "");
    grows(
        "src/o.js",
        &base,
        &express(
            "db",
            "",
            "",
            "  for (const item of order.items) audit.log(item);\n",
        ),
        "was 17, now 18",
    );
    let go = |runs: &[&str]| {
        let body: String = runs
            .iter()
            .map(|name| format!("\tt.Run(\"{name}\", func(t *testing.T) {{ if err != nil && x {{ t.Fatal(err) }}; if got != want || y {{ t.Fail() }} }})\n"))
            .collect();
        format!("package store\nfunc TestStore(t *testing.T) {{\n{body}}}\n")
    };
    let four = go(&["a", "a", "b", "c"]);
    let added = format!(
        "{}\tt.Run(\"d\", func(t *testing.T) {{ if err != nil {{ t.Fatal(err) }} }})\n}}\n",
        four.trim_end().trim_end_matches('}')
    );
    grows("store_test.go", &four, &added, "'TestStore'");
    let fastify = |routes: &[&str], extra: &str| {
        let body: String = routes
            .iter()
            .map(|route| format!("  fastify.get('{route}', async (req) => {{ if (req.q && req.r) {{ return 1; }} if (req.s || req.t) {{ return 2; }} return 0; }});\n"))
            .collect();
        format!("module.exports = async function (fastify, opts) {{\n{body}{extra}}};\n")
    };
    let extra = "  fastify.get('/d', async (req) => { if (req.q) { return 1; } return 0; });\n";
    grows(
        "src/plugin.js",
        &fastify(&["/a", "/a", "/b", "/c"], ""),
        &fastify(&["/a", "/a", "/b", "/c"], extra),
        "'module.exports'",
    );
}

#[test]
fn only_functions_the_patch_touched_are_judged() {
    let untouched = format!("function big() {{\n{}}}\n", eight("  "));
    // A grandfathered over-cap function is not judged when the patch
    // edits something else, even if pairing could not find it.
    let base = format!("{untouched}function small() {{ return 1; }}\n");
    let post = format!("{untouched}function small() {{ return 2; }}\n");
    accepts("src/f.js", &base, &post);
    // A new file has every line added.
    assert!(validate_complexity("src/f.js", None, &untouched, MAX).is_err());
}
