//! Bounded visual analysis on the WAV writer thread, never on the device callback.
pub const BANDS: usize = 48;
const WINDOW: usize = 1024;
pub struct Spectrum {
    samples: [f32; WINDOW],
    cursor: usize,
    since_frame: usize,
    frame: usize,
    channels: usize,
    sum: f32,
    channel: usize,
    stride: usize,
    coefficients: [f32; BANDS],
    window: [f32; WINDOW],
}
impl Spectrum {
    pub fn new(rate: u32, channels: u16) -> Self {
        let stride = (rate as usize / 16000).max(1);
        let rate = rate as f32 / stride as f32;
        Self {
            samples: [0.; WINDOW],
            cursor: 0,
            since_frame: 0,
            frame: 0,
            channels: channels as usize,
            sum: 0.,
            channel: 0,
            stride,
            coefficients: std::array::from_fn(|i| {
                let frequency =
                    80. * (6000_f32.min(rate * 0.45) / 80.).powf(i as f32 / (BANDS - 1) as f32);
                2. * (std::f32::consts::TAU * frequency / rate).cos()
            }),
            window: std::array::from_fn(|i| {
                0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (WINDOW - 1) as f32).cos()
            }),
        }
    }
    pub fn push(&mut self, sample: f32) -> Option<[f32; BANDS]> {
        self.sum += if sample.is_finite() { sample } else { 0. };
        self.channel += 1;
        if self.channel < self.channels {
            return None;
        }
        let mono = self.sum / self.channels as f32;
        self.sum = 0.;
        self.channel = 0;
        self.frame += 1;
        if !self.frame.is_multiple_of(self.stride) {
            return None;
        }
        self.samples[self.cursor] = mono;
        self.cursor = (self.cursor + 1) % WINDOW;
        self.since_frame += 1;
        if self.since_frame < 512 {
            return None;
        }
        self.since_frame = 0;
        Some(std::array::from_fn(|band| {
            let coefficient = self.coefficients[band];
            let (mut a, mut b) = (0., 0.);
            for i in 0..WINDOW {
                let next =
                    self.samples[(self.cursor + i) % WINDOW] * self.window[i] + coefficient * a - b;
                b = a;
                a = next;
            }
            let amplitude =
                (a * a + b * b - coefficient * a * b).max(0.).sqrt() * 4. / WINDOW as f32;
            ((20. * amplitude.max(0.000001).log10() + 65.) / 55.).clamp(0., 1.)
        }))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn tone(frequency: f32) -> [f32; BANDS] {
        let mut analyzer = Spectrum::new(48000, 2);
        let mut bands = [0.; BANDS];
        for i in 0..48000 {
            let sample = 0.1 * (std::f32::consts::TAU * frequency * i as f32 / 48000.).sin();
            for _ in 0..2 {
                if let Some(next) = analyzer.push(sample) {
                    bands = next;
                }
            }
        }
        bands
    }
    #[test]
    fn silence_is_quiet_and_frequency_changes_move_the_peak() {
        assert_eq!(tone(0.), [0.; BANDS]);
        let peak = |bands: [f32; BANDS]| {
            bands
                .into_iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap()
                .0
        };
        let low = peak(tone(250.));
        let high = peak(tone(2000.));
        assert!((10..16).contains(&low), "{low}");
        assert!((32..39).contains(&high), "{high}");
    }
}
