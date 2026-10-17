use super::*;

#[tokio::test]
async fn a_changed_input_invalidates_exactly_the_affected_unit() {
    let temp = corpus();
    let cwd = temp.path();
    complete(cwd, Critic::new(), &FreezeResume::none()).await;
    let path = cwd.join("tasks/PRD-QX-001/TASK-QX-002.md");
    let edited = std::fs::read_to_string(&path).unwrap() + "\nAlso sorts the catalogue.\n";
    std::fs::write(&path, edited).unwrap();
    let critic = Critic::new();
    complete(cwd, critic.clone(), &FreezeResume::none()).await;
    assert_eq!(critic.asked(), vec![ids(&[3, 4])]);

    let prd = cwd.join("tasks/PRD-QX-001.md");
    let text = std::fs::read_to_string(&prd)
        .unwrap()
        .replace("lists gadget 5", "lists gadget 5 with its price");
    std::fs::write(&prd, text).unwrap();
    let critic = Critic::new();
    complete(cwd, critic.clone(), &FreezeResume::none()).await;
    assert_eq!(
        critic.asked(),
        vec![ids(&[1, 2]), ids(&[3, 4]), ids(&[5, 6])],
        "the edited PRD source section supplies context to all six obligations"
    );
}
