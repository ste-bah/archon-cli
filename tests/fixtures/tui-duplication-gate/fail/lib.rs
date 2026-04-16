// Fail fixture — contains 30+ line identical blocks triggering jscpd threshold

pub fn helper_function_a() {
    let mut data = Vec::new();
    data.push("item1");
    data.push("item2");
    data.push("item3");
    data.push("item4");
    data.push("item5");
    data.push("item6");
    data.push("item7");
    data.push("item8");
    data.push("item9");
    data.push("item10");
    data.push("item11");
    data.push("item12");
    data.push("item13");
    data.push("item14");
    data.push("item15");
    data.push("item16");
    data.push("item17");
    data.push("item18");
    data.push("item19");
    data.push("item20");
    data.push("item21");
    data.push("item22");
    data.push("item23");
    data.push("item24");
    data.push("item25");
    data.push("item26");
    data.push("item27");
    data.push("item28");
    data.push("item29");
    data.push("item30");
    let _ = data.len();
}

pub fn helper_function_b() {
    let mut data = Vec::new();
    data.push("item1");
    data.push("item2");
    data.push("item3");
    data.push("item4");
    data.push("item5");
    data.push("item6");
    data.push("item7");
    data.push("item8");
    data.push("item9");
    data.push("item10");
    data.push("item11");
    data.push("item12");
    data.push("item13");
    data.push("item14");
    data.push("item15");
    data.push("item16");
    data.push("item17");
    data.push("item18");
    data.push("item19");
    data.push("item20");
    data.push("item21");
    data.push("item22");
    data.push("item23");
    data.push("item24");
    data.push("item25");
    data.push("item26");
    data.push("item27");
    data.push("item28");
    data.push("item29");
    data.push("item30");
    let _ = data.len();
}

pub fn unique_function() {
    println!("This is unique code that differs from the duplicated blocks");
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_a() {
        assert!(true);
    }
}
