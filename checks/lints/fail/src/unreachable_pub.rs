mod inner {
    pub fn g() {}
}

pub fn f() {
    inner::g();
}
