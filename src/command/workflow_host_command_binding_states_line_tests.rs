use super::*;

#[test]
fn binding_states_preserve_body_line_endings_and_final_newline() {
    for newline in ["\n", "\r\n"] {
        for trailing in [false, true] {
            let text = format!(
                "{newline}First{newline}Last{}",
                if trailing { newline } else { "" }
            );
            let states = [("id".into(), "undecided".into())].into();
            let stamped = body(&text, &states);
            assert!(stamped.ends_with(&text[newline.len()..]));
            assert_eq!(stamped.ends_with(newline), trailing);
            if newline == "\r\n" {
                assert!(!stamped.replace("\r\n", "").contains('\n'));
            }
            assert_eq!(body(&stamped, &states), stamped);
        }
    }
}
