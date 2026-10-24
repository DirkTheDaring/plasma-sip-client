//! Pure Rust implementation of ITU-T Recommendation G.711 (PCMA & PCMU).
//!
//! Provides conversion between 16-bit linear PCM and 8-bit A-law / μ-law samples.

/// Encodes a 16-bit signed linear PCM sample to 8-bit G.711 A-law (PCMA).
#[inline]
pub fn linear_to_alaw(mut pcm: i16) -> u8 {
    let sign = if pcm < 0 {
        pcm = if pcm == i16::MIN { i16::MAX } else { -pcm };
        0x00
    } else {
        0x80
    };

    let exponent = if pcm >= 2048 {
        if pcm >= 8192 {
            if pcm >= 16384 {
                7
            } else {
                6
            }
        } else if pcm >= 4096 {
            5
        } else {
            4
        }
    } else if pcm >= 512 {
        if pcm >= 1024 {
            3
        } else {
            2
        }
    } else if pcm >= 256 {
        1
    } else {
        0
    };

    let mantissa = if exponent == 0 {
        (pcm >> 4) & 0x0F
    } else {
        (pcm >> (exponent + 3)) & 0x0F
    };

    let alaw = sign | (exponent << 4) | (mantissa as u8);
    // Invert even bits according to ITU-T G.711 spec
    alaw ^ 0x55
}

/// Decodes an 8-bit G.711 A-law (PCMA) sample to a 16-bit signed linear PCM sample.
#[inline]
pub fn alaw_to_linear(mut alaw: u8) -> i16 {
    // Invert even bits
    alaw ^= 0x55;

    let sign = (alaw & 0x80) != 0;
    let exponent = ((alaw & 0x70) >> 4) as i32;
    let mantissa = (alaw & 0x0F) as i32;

    let sample = if exponent == 0 {
        (mantissa << 4) + 8
    } else {
        ((mantissa << 4) + 0x108) << (exponent - 1)
    };

    if sign {
        sample as i16
    } else {
        -(sample as i16)
    }
}

/// Encodes a 16-bit signed linear PCM sample to 8-bit G.711 μ-law (PCMU).
#[inline]
pub fn linear_to_ulaw(mut pcm: i16) -> u8 {
    const BIAS: i16 = 0x84;
    const CLIP: i16 = 32635;

    let sign = (pcm >> 8) & 0x80;
    if sign != 0 {
        pcm = if pcm == i16::MIN { i16::MAX } else { -pcm };
    }
    if pcm > CLIP {
        pcm = CLIP;
    }
    pcm += BIAS;

    let mut exponent = 7;
    let mut mask = 0x4000;
    while (pcm & mask) == 0 && exponent > 0 {
        exponent -= 1;
        mask >>= 1;
    }

    let mantissa = (pcm >> (exponent + 3)) & 0x0F;
    let ulaw = (sign as u8) | (exponent << 4) | (mantissa as u8);
    !ulaw
}

/// Decodes an 8-bit G.711 μ-law (PCMU) sample to a 16-bit signed linear PCM sample.
#[inline]
pub fn ulaw_to_linear(ulaw: u8) -> i16 {
    const BIAS: i32 = 0x84;
    let ulaw = !ulaw;
    let sign = (ulaw & 0x80) != 0;
    let exponent = ((ulaw & 0x70) >> 4) as i32;
    let mantissa = (ulaw & 0x0F) as i32;

    let mut sample = ((mantissa << 3) + BIAS) << exponent;
    sample -= BIAS;

    if sign {
        -(sample as i16)
    } else {
        sample as i16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alaw_roundtrip_quantization() {
        // Test zero crossing
        let a_zero = linear_to_alaw(0);
        let pcm_zero = alaw_to_linear(a_zero);
        assert!(
            pcm_zero.abs() <= 16,
            "Error too large for 0: got {}",
            pcm_zero
        );

        // Test various amplitudes
        for val in [-30000, -15000, -1000, -100, 100, 1000, 15000, 30000] {
            let encoded = linear_to_alaw(val);
            let decoded = alaw_to_linear(encoded);
            let error = (decoded as i32 - val as i32).abs();
            // G.711 quantization error is bounded by ~3.2%
            let max_expected_error = (val.abs() as i32 / 16) + 32;
            assert!(
                error <= max_expected_error,
                "Error {} too high for input {}",
                error,
                val
            );
        }
    }

    #[test]
    fn test_ulaw_roundtrip_quantization() {
        for val in [-30000, -15000, -1000, -100, 0, 100, 1000, 15000, 30000] {
            let encoded = linear_to_ulaw(val);
            let decoded = ulaw_to_linear(encoded);
            let error = (decoded as i32 - val as i32).abs();
            let max_expected_error = (val.abs() as i32 / 16) + 64;
            assert!(
                error <= max_expected_error,
                "Error {} too high for input {}",
                error,
                val
            );
        }
    }

    #[test]
    fn test_g711_known_answer_vectors() {
        // ITU-T G.711 A-law known vectors
        assert_eq!(linear_to_alaw(0), 0xD5);
        assert_eq!(alaw_to_linear(0xD5), 8);
        assert_eq!(linear_to_alaw(-8), 0x55);
        assert_eq!(alaw_to_linear(0x55), -8);
        assert_eq!(linear_to_alaw(32767), 0xAA);
        assert_eq!(alaw_to_linear(0xAA), 32256);
        assert_eq!(linear_to_alaw(-32768), 0x2A);
        assert_eq!(alaw_to_linear(0x2A), -32256);

        // ITU-T G.711 mu-law known vectors
        assert_eq!(linear_to_ulaw(0), 0xFF);
        assert_eq!(ulaw_to_linear(0xFF), 0);
        assert_eq!(linear_to_ulaw(32767), 0x80);
        assert_eq!(ulaw_to_linear(0x80), 32124);
        assert_eq!(linear_to_ulaw(-32768), 0x00);
        assert_eq!(ulaw_to_linear(0x00), -32124);
    }
}
