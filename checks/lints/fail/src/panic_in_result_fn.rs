pub fn f(x: u8) -> Result<u8, String> {
    assert!(x > 0, "x is positive");
    Ok(x)
}
