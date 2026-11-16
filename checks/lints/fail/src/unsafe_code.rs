pub fn f(x: u32) -> f32 { unsafe { std::mem::transmute::<u32, f32>(x) } }
