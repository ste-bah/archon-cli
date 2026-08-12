//! Java indexing for the cartographer (#176).
//!
//! Held apart from `cartographer_tests.rs` because Java needs handling the
//! other five languages do not — nested type names, annotations inside the
//! declaration node, and imports that resolve exactly rather than by guess —
//! so its assertions are worth reading together.

use archon_tools::cartographer::deps::extract_dependencies;
use archon_tools::cartographer::index::{Symbol, SymbolKind};
use archon_tools::cartographer::parser::{language_for_file, parse_file};

/// One fixture exercising every declaration form the Java extractor claims to
/// handle, so the assertions below all describe the same file rather than each
/// inventing a snippet that happens to work.
const JAVA_FIXTURE: &str = r#"
package com.example.orders;

import java.util.List;
import java.util.concurrent.*;
import static org.junit.Assert.assertEquals;

public @interface Audited {
    String value();
}

public interface Pricing {
    long priceOf(String sku);
}

public enum Currency {
    GBP,
    USD
}

public record LineItem(String sku, int quantity) {}

public class OrderService implements Pricing {

    public OrderService(List<String> skus) {
    }

    @Override
    public long priceOf(String sku) {
        return 0L;
    }

    public static final class Builder {
        public OrderService build() {
            return null;
        }
    }
}
"#;

fn java_fixture_symbols() -> Vec<Symbol> {
    parse_file(
        "src/main/java/com/example/orders/OrderService.java",
        JAVA_FIXTURE,
        "java",
    )
}

fn find_java_symbol<'a>(syms: &'a [Symbol], name: &str) -> &'a Symbol {
    syms.iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no symbol named {name}; got: {syms:?}"))
}

// ---------------------------------------------------------------------------
// Declaration forms
// ---------------------------------------------------------------------------

#[test]
fn language_detected_for_java() {
    assert_eq!(language_for_file("Foo.java"), Some("java"));
}

#[test]
fn java_class_extracted() {
    let syms = java_fixture_symbols();
    assert_eq!(
        find_java_symbol(&syms, "OrderService").kind,
        SymbolKind::Class
    );
}

#[test]
fn java_interface_extracted() {
    let syms = java_fixture_symbols();
    assert_eq!(
        find_java_symbol(&syms, "Pricing").kind,
        SymbolKind::Interface
    );
}

#[test]
fn java_enum_extracted() {
    let syms = java_fixture_symbols();
    assert_eq!(find_java_symbol(&syms, "Currency").kind, SymbolKind::Enum);
}

/// A record is its own kind rather than `Struct`: the summary names the
/// construct a Java reader is looking for.
#[test]
fn java_record_extracted() {
    let syms = java_fixture_symbols();
    assert_eq!(find_java_symbol(&syms, "LineItem").kind, SymbolKind::Record);
}

#[test]
fn java_annotation_type_extracted() {
    let syms = java_fixture_symbols();
    assert_eq!(
        find_java_symbol(&syms, "Audited").kind,
        SymbolKind::Annotation
    );
}

#[test]
fn java_constructor_extracted_as_method() {
    let syms = java_fixture_symbols();
    assert_eq!(
        find_java_symbol(&syms, "OrderService.OrderService").kind,
        SymbolKind::Method
    );
}

// ---------------------------------------------------------------------------
// Nested-type qualification
// ---------------------------------------------------------------------------

/// The walker finds a nested type but has no parent context of its own, so
/// without the enclosing-type chain `Builder` would be indexed as a top-level
/// name — the identity a reader needs is `OrderService.Builder`.
#[test]
fn java_nested_class_keeps_its_enclosing_type() {
    let syms = java_fixture_symbols();
    assert_eq!(
        find_java_symbol(&syms, "OrderService.Builder").kind,
        SymbolKind::Class
    );
    assert!(
        !syms.iter().any(|s| s.name == "Builder"),
        "nested type should not also be indexed unqualified: {syms:?}"
    );
}

/// Java method names repeat freely across classes, so a bare `build` would not
/// identify anything. Methods carry the same qualification as nested types.
#[test]
fn java_methods_are_qualified_by_enclosing_type() {
    let syms = java_fixture_symbols();
    assert_eq!(
        find_java_symbol(&syms, "OrderService.Builder.build").kind,
        SymbolKind::Method
    );
    assert_eq!(
        find_java_symbol(&syms, "Pricing.priceOf").kind,
        SymbolKind::Method
    );
}

// ---------------------------------------------------------------------------
// Signatures
// ---------------------------------------------------------------------------

/// tree-sitter puts annotations inside the declaration node, so a first-line
/// signature rule would record `@Override` for this method.
#[test]
fn java_signature_starts_after_annotations() {
    let syms = java_fixture_symbols();
    let sig = &find_java_symbol(&syms, "OrderService.priceOf").signature;
    assert_eq!(sig, "public long priceOf(String sku)", "got: {sig}");
}

#[test]
fn java_signature_excludes_the_body() {
    let syms = java_fixture_symbols();
    let sig = &find_java_symbol(&syms, "OrderService").signature;
    assert_eq!(
        sig, "public class OrderService implements Pricing",
        "got: {sig}"
    );
}

#[test]
fn malformed_java_source_does_not_panic() {
    let syms = parse_file("Bad.java", "class class { void void ((( }", "java");
    let _ = syms.len();
}

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

/// Java imports are fully qualified, so these names are exact rather than a
/// guess at what a relative path resolves to.
#[test]
fn java_imports_extracted() {
    let deps = extract_dependencies(JAVA_FIXTURE, "java");
    assert!(
        deps.contains(&"java.util.List".to_string()),
        "plain import missing: {deps:?}"
    );
}

/// `import java.util.concurrent.*;` names the package, not a type, so the
/// trailing `.*` must not become part of the name.
#[test]
fn java_on_demand_import_names_the_package() {
    let deps = extract_dependencies(JAVA_FIXTURE, "java");
    assert!(
        deps.contains(&"java.util.concurrent".to_string()),
        "on-demand import should yield the package: {deps:?}"
    );
}

/// A static import ends in a member name; the edge should point at the type
/// that declares it.
#[test]
fn java_static_import_drops_the_member() {
    let deps = extract_dependencies(JAVA_FIXTURE, "java");
    assert!(
        deps.contains(&"org.junit.Assert".to_string()),
        "static import should yield the declaring type: {deps:?}"
    );
    assert!(
        !deps.iter().any(|d| d.ends_with("assertEquals")),
        "member name should not survive: {deps:?}"
    );
}
