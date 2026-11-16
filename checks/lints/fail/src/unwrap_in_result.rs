pub fn f(input: &str) -> Result<(), String> {
    let value = input.parse::<i32>().expect("number");
    if value % 3 != 0 {
        Err("not divisible")?;
    }
    Ok(())
}
