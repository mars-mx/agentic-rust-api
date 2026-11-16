pub fn f(s: &str) -> Result<u8, String> {
    s.parse::<u8>().map_err(|_| String::from("bad"))
}
