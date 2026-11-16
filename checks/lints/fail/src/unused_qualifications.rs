use std::mem::swap;

pub fn f(a: &mut u8, b: &mut u8) {
    swap(a, b);
    std::mem::swap(a, b);
}
