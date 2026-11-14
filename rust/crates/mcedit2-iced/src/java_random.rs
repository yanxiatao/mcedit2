//! 模型随机选择与原版噪声共用的 Java 48 位随机序列。
pub(crate) struct LegacyRandom {
    seed: u64,
}
impl LegacyRandom {
    pub(crate) fn new(seed: i64) -> Self {
        Self {
            seed: (seed as u64 ^ 0x5deece66d) & ((1 << 48) - 1),
        }
    }
    fn bits(&mut self, count: u32) -> u32 {
        self.seed = self.seed.wrapping_mul(0x5deece66d).wrapping_add(11) & ((1 << 48) - 1);
        (self.seed >> (48 - count)) as u32
    }
    pub(crate) fn next_long(&mut self) -> i64 {
        // Java 的低位 int 也带符号，不能直接拼接两个无符号数。
        (i64::from(self.bits(32) as i32) << 32).wrapping_add(i64::from(self.bits(32) as i32))
    }
    pub(crate) fn bounded(&mut self, bound: u32) -> u32 {
        assert!(
            bound > 0 && bound <= i32::MAX as u32,
            "Java 随机范围必须为正 int"
        );
        if bound.is_power_of_two() {
            return (u64::from(bound) * u64::from(self.bits(31)) >> 31) as u32;
        }
        loop {
            let bits = self.bits(31);
            let value = bits % bound;
            if (bits as i32)
                .wrapping_sub(value as i32)
                .wrapping_add(bound as i32 - 1)
                >= 0
            {
                return value;
            }
        }
    }
    pub(crate) fn float(&mut self) -> f32 {
        self.bits(24) as f32 / (1_u32 << 24) as f32
    }
    pub(crate) fn double(&mut self) -> f64 {
        let bits = (u64::from(self.bits(26)) << 27) + u64::from(self.bits(27));
        bits as f64 / (1_u64 << 53) as f64
    }
}
