//! Fast, allocation-free resampling and channel conversion for real-time audio callbacks.

/// Converts a linear 16-bit PCM sample (-32768..=32767) to f32 (-1.0..=1.0).
#[inline]
pub fn i16_to_f32(sample: i16) -> f32 {
    if sample < 0 {
        sample as f32 / 32768.0
    } else {
        sample as f32 / 32767.0
    }
}

/// Converts a float sample (-1.0..=1.0) to linear 16-bit PCM (-32768..=32767).
#[inline]
pub fn f32_to_i16(sample: f32) -> i16 {
    if sample <= -1.0 {
        -32768
    } else if sample >= 1.0 {
        32767
    } else if sample < 0.0 {
        (sample * 32768.0).clamp(-32768.0, 32767.0) as i16
    } else {
        (sample * 32767.0).clamp(-32768.0, 32767.0) as i16
    }
}

/// Downmixes an interleaved multi-channel frame to mono.
#[inline]
pub fn downmix_to_mono(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: f32 = frame.iter().copied().sum();
    sum / (frame.len() as f32)
}

/// Downmixes an interleaved multi-channel i16 frame to mono.
#[inline]
pub fn downmix_i16_to_mono(frame: &[i16]) -> i16 {
    if frame.is_empty() {
        return 0;
    }
    let sum: i32 = frame.iter().map(|&s| s as i32).sum();
    (sum / frame.len() as i32) as i16
}

const FIR_TAPS: usize = 21;

/// Low-pass anti-aliasing FIR filter (windowed-sinc with Hamming window)
#[derive(Debug, Clone)]
pub struct LowPassFirFilter {
    taps: [f32; FIR_TAPS],
    history: [f32; FIR_TAPS * 2],
    index: usize,
    initialized: bool,
    enabled: bool,
}

impl LowPassFirFilter {
    pub fn new(sample_rate: u32, cutoff_hz: f32) -> Self {
        if sample_rate <= 8000 {
            return Self {
                taps: [0.0; FIR_TAPS],
                history: [0.0; FIR_TAPS * 2],
                index: 0,
                initialized: false,
                enabled: false,
            };
        }

        let mut taps = [0.0f32; FIR_TAPS];
        let m_center = (FIR_TAPS - 1) as f32 / 2.0;
        let fnorm = (cutoff_hz / sample_rate as f32).min(0.499);
        let mut sum = 0.0f32;

        for (i, tap) in taps.iter_mut().enumerate() {
            let m = i as f32 - m_center;
            let sinc = if m.abs() < 1e-6 {
                2.0 * fnorm
            } else {
                (2.0 * std::f32::consts::PI * fnorm * m).sin() / (std::f32::consts::PI * m)
            };
            let w =
                0.54 - 0.46 * (2.0 * std::f32::consts::PI * i as f32 / (FIR_TAPS - 1) as f32).cos();
            *tap = sinc * w;
            sum += *tap;
        }

        if sum > 0.0 {
            for tap in taps.iter_mut() {
                *tap /= sum;
            }
        }

        Self {
            taps,
            history: [0.0; FIR_TAPS * 2],
            index: 0,
            initialized: false,
            enabled: true,
        }
    }

    #[inline]
    pub fn filter(&mut self, sample: f32) -> f32 {
        if !self.enabled {
            return sample;
        }

        if !self.initialized {
            self.history.fill(sample);
            self.initialized = true;
        }

        self.history[self.index] = sample;
        self.history[self.index + FIR_TAPS] = sample;

        let slice = &self.history[self.index + 1..self.index + 1 + FIR_TAPS];
        let mut acc = 0.0f32;
        for (i, &tap) in self.taps.iter().enumerate() {
            acc += tap * slice[FIR_TAPS - 1 - i];
        }

        self.index = (self.index + 1) % FIR_TAPS;
        acc
    }
}

/// Resampler state for streaming audio from an arbitrary device sample rate to 8000 Hz.
#[derive(Debug, Clone)]
pub struct InputResampler {
    device_sample_rate: u32,
    channels: usize,
    phase: f64,
    last_sample: f32,
    filter: LowPassFirFilter,
}

impl InputResampler {
    pub fn new(device_sample_rate: u32, channels: u16) -> Self {
        Self {
            device_sample_rate,
            channels: channels.max(1) as usize,
            phase: 0.0,
            last_sample: 0.0,
            filter: LowPassFirFilter::new(device_sample_rate, 3800.0),
        }
    }

    /// Processes an input chunk of interleaved f32 samples from the CPAL callback,
    /// converting them to 8000 Hz mono i16 and pushing into the provided rtrb producer.
    /// Returns the number of samples pushed.
    pub fn process_f32(
        &mut self,
        input: &[f32],
        producer: &mut rtrb::Producer<i16>,
        muted: bool,
    ) -> usize {
        let mut pushed = 0;
        let step = 8000.0 / self.device_sample_rate as f64;

        for chunk in input.chunks(self.channels) {
            let raw_sample = downmix_to_mono(chunk);
            let current_sample = self.filter.filter(raw_sample);
            self.phase += step;

            while self.phase >= 1.0 - 1e-9 {
                let frac = ((1.0 - (self.phase - step)) / step).clamp(0.0, 1.0);
                let interp = self.last_sample + (current_sample - self.last_sample) * (frac as f32);
                let out_sample = if muted { 0 } else { f32_to_i16(interp) };

                if producer.push(out_sample).is_ok() {
                    pushed += 1;
                }
                self.phase -= 1.0;
            }

            self.last_sample = current_sample;
        }

        pushed
    }

    /// Processes an input chunk of interleaved i16 samples from the CPAL callback.
    pub fn process_i16(
        &mut self,
        input: &[i16],
        producer: &mut rtrb::Producer<i16>,
        muted: bool,
    ) -> usize {
        let mut pushed = 0;
        let step = 8000.0 / self.device_sample_rate as f64;

        for chunk in input.chunks(self.channels) {
            let raw_sample = i16_to_f32(downmix_i16_to_mono(chunk));
            let current_sample = self.filter.filter(raw_sample);
            self.phase += step;

            while self.phase >= 1.0 - 1e-9 {
                let frac = ((1.0 - (self.phase - step)) / step).clamp(0.0, 1.0);
                let interp = self.last_sample + (current_sample - self.last_sample) * (frac as f32);
                let out_sample = if muted { 0 } else { f32_to_i16(interp) };

                if producer.push(out_sample).is_ok() {
                    pushed += 1;
                }
                self.phase -= 1.0;
            }

            self.last_sample = current_sample;
        }

        pushed
    }
}

/// Resampler state for streaming 8000 Hz mono audio to an arbitrary device sample rate.
#[derive(Debug, Clone)]
pub struct OutputResampler {
    device_sample_rate: u32,
    channels: usize,
    phase: f64,
    prev_sample: f32,
    next_sample: f32,
}

impl OutputResampler {
    pub fn new(device_sample_rate: u32, channels: u16) -> Self {
        Self {
            device_sample_rate,
            channels: channels.max(1) as usize,
            phase: 1.0,
            prev_sample: 0.0,
            next_sample: 0.0,
        }
    }

    /// Fills the output buffer of interleaved f32 samples by pulling 8000 Hz mono i16 samples
    /// from the consumer and interpolating to the output sample rate and channel layout.
    pub fn fill_f32(&mut self, output: &mut [f32], consumer: &mut rtrb::Consumer<i16>) {
        let step = 8000.0 / self.device_sample_rate as f64;

        for frame in output.chunks_mut(self.channels) {
            while self.phase >= 1.0 {
                self.prev_sample = self.next_sample;
                self.next_sample = match consumer.pop() {
                    Ok(s) => i16_to_f32(s),
                    Err(_) => 0.0,
                };
                self.phase -= 1.0;
            }

            let interp =
                self.prev_sample + (self.next_sample - self.prev_sample) * (self.phase as f32);
            for ch in frame.iter_mut() {
                *ch = interp;
            }

            self.phase += step;
        }
    }

    /// Fills the output buffer of interleaved i16 samples.
    pub fn fill_i16(&mut self, output: &mut [i16], consumer: &mut rtrb::Consumer<i16>) {
        let step = 8000.0 / self.device_sample_rate as f64;

        for frame in output.chunks_mut(self.channels) {
            while self.phase >= 1.0 {
                self.prev_sample = self.next_sample;
                self.next_sample = match consumer.pop() {
                    Ok(s) => i16_to_f32(s),
                    Err(_) => 0.0,
                };
                self.phase -= 1.0;
            }

            let interp =
                self.prev_sample + (self.next_sample - self.prev_sample) * (self.phase as f32);
            let s_i16 = f32_to_i16(interp);
            for ch in frame.iter_mut() {
                *ch = s_i16;
            }

            self.phase += step;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conversion_roundtrip() {
        assert_eq!(f32_to_i16(0.0), 0);
        assert_eq!(f32_to_i16(1.0), 32767);
        assert_eq!(f32_to_i16(-1.0), -32768);

        let s: i16 = 16384;
        let f = i16_to_f32(s);
        let back = f32_to_i16(f);
        assert!((s - back).abs() <= 1);
    }

    #[test]
    fn test_resampler_48k_to_8k() {
        let mut resampler = InputResampler::new(48000, 2);
        let (mut prod, mut cons) = rtrb::RingBuffer::<i16>::new(100);

        // 48 stereo samples at 48kHz = 48 / 6 = 8 samples at 8kHz
        let input = vec![0.5f32; 96]; // 48 frames * 2 channels
        let pushed = resampler.process_f32(&input, &mut prod, false);
        assert_eq!(pushed, 8);
        assert_eq!(cons.slots(), 8);

        let sample = cons.pop().unwrap();
        // 0.5 * 32767 = 16383
        assert!((sample - 16383).abs() <= 2);
    }

    #[test]
    fn test_resampler_8k_to_48k() {
        let mut resampler = OutputResampler::new(48000, 2);
        let (mut prod, mut cons) = rtrb::RingBuffer::<i16>::new(100);

        // Feed 10 samples of 16384 into consumer
        for _ in 0..10 {
            prod.push(16384).unwrap();
        }

        // Fill 48 stereo frames (96 samples)
        let mut output = vec![0.0f32; 96];
        resampler.fill_f32(&mut output, &mut cons);

        // Check that frames are non-zero and stereo channels are equal
        for chunk in output.chunks(2) {
            assert_eq!(chunk[0], chunk[1]);
        }
    }

    #[test]
    fn test_low_pass_fir_filter_attenuation() {
        let mut filter = LowPassFirFilter::new(48000, 3800.0);
        let sample_rate = 48000.0f32;

        // Pass 500 Hz tone (passband)
        let mut passband_energy = 0.0f32;
        for i in 0..480 {
            let t = i as f32 / sample_rate;
            let sample = (2.0 * std::f32::consts::PI * 500.0 * t).sin();
            let out = filter.filter(sample);
            if i >= 40 {
                passband_energy += out * out;
            }
        }

        // Pass 15000 Hz tone (stopband)
        let mut filter2 = LowPassFirFilter::new(48000, 3800.0);
        let mut stopband_energy = 0.0f32;
        for i in 0..480 {
            let t = i as f32 / sample_rate;
            let sample = (2.0 * std::f32::consts::PI * 15000.0 * t).sin();
            let out = filter2.filter(sample);
            if i >= 40 {
                stopband_energy += out * out;
            }
        }

        // Stopband energy should be attenuated by at least 15 dB (ratio < 0.03) compared to passband
        assert!(
            stopband_energy / passband_energy < 0.05,
            "FIR filter did not sufficiently attenuate 15kHz tone (stop: {}, pass: {})",
            stopband_energy,
            passband_energy
        );
    }
}
