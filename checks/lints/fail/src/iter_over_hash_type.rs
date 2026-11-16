pub fn f(m: &std::collections::HashMap<u8, u8>) -> u32 {
    let mut total = 0u32;
    for value in m.values() {
        total += u32::from(*value);
    }
    total
}
