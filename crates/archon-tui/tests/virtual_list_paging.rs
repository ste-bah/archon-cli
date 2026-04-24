use archon_tui::virtual_list::VirtualList;

#[test]
fn move_up_wraps_to_last() {
    let items = vec!["a", "b", "c"];
    let mut list = VirtualList::new(items, 3);
    list.move_up();
    assert_eq!(list.selected_index(), 2); // wrapped to last
}

#[test]
fn move_down_wraps_to_first() {
    let items = vec!["a", "b", "c"];
    let mut list = VirtualList::new(items, 3);
    list.move_down();
    assert_eq!(list.selected_index(), 1); // moved to 1
}

#[test]
fn page_up_down() {
    let items: Vec<i32> = (0..20).collect();
    let mut list = VirtualList::new(items, 5);
    list.move_down();
    list.move_down();
    list.page_up();
    assert!(list.selected_index() < 5);
}

#[test]
fn empty_list() {
    let items: Vec<i32> = vec![];
    let list = VirtualList::new(items, 5);
    assert!(list.is_empty());
    assert_eq!(list.len(), 0);
}
