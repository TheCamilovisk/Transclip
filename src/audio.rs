//! Audio normalization (architecture section 12, decision D4).
//!
//! Pure, hardware-free DSP: sample-format conversion to `f32`, multi-channel
//! downmix to mono, and linear-interpolation resampling to the Whisper target
//! rate. Nothing here touches CPAL or any device; the recorder applies these
//! functions to completed captures outside the audio callback.

/// Whisper's target sample rate (whisper-rs-sys 0.15.0 / whisper.cpp 1.8.3,
/// `WHISPER_SAMPLE_RATE = 16000`; decision D4).
pub const WHISPER_SAMPLE_RATE: u32 = 16_000;

/// Converts a slice of samples in any dasp-supported format to `f32`.
///
/// Used by the recorder's data callback for the device's numeric format
/// (I8/I16/I24/I32/U8/U16/U24/U32/F32/F64 — the supported set validated at
/// `start`; decision D4). Integer formats map `-1.0..1.0` onto their range;
/// floats pass through.
pub fn convert_to_f32<T>(samples: &[T]) -> Vec<f32>
where
    T: dasp_sample::Sample + dasp_sample::ToSample<f32> + Copy,
{
    samples.iter().map(|&s| s.to_sample::<f32>()).collect()
}

/// Downmixes interleaved multi-channel samples to mono by per-frame averaging
/// (decision D4). Mono input is returned unchanged (copied). A trailing
/// partial frame is dropped, which cannot happen from CPAL streams that
/// deliver whole frames.
pub fn downmix_to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    let channels = channels as usize;
    let frames = samples.len() / channels;
    let mut mono = Vec::with_capacity(frames);
    for frame in samples.chunks_exact(channels) {
        mono.push(frame.iter().sum::<f32>() / channels as f32);
    }
    mono
}

/// Resamples mono `f32` samples from `from_rate` to `to_rate` using linear
/// interpolation (decision D4: minimal, deterministic, no new dependency).
///
/// - Equal rates and empty input are returned unchanged.
/// - Output length is `len * to / from` (floor), and each output sample is
///   the linear interpolation of the two nearest source samples.
pub fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if samples.is_empty() || from_rate == to_rate {
        return samples.to_vec();
    }
    if from_rate == 0 || to_rate == 0 {
        return Vec::new();
    }
    let out_len = (samples.len() as f64 * (to_rate as f64 / from_rate as f64)) as usize;
    if out_len == 0 {
        return Vec::new();
    }
    let step = from_rate as f64 / to_rate as f64; // source position per output sample
    let last = samples.len() - 1;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * step;
        let idx = (pos.floor() as usize).min(last);
        let frac = (pos - idx as f64) as f32;
        let a = samples[idx];
        let b = samples[(idx + 1).min(last)];
        out.push(a + (b - a) * frac);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dasp_sample::Sample as _;

    // ---- Sample conversion fixtures (decision D4) ----

    #[test]
    fn converts_i16_to_f32() {
        let out = convert_to_f32(&[i16::MIN, 0, i16::MAX]);
        assert_eq!(out[0], -1.0);
        assert_eq!(out[1], 0.0);
        assert!((out[2] - 0.999_969_5).abs() < 1e-6); // 32767 / 32768
    }

    #[test]
    fn converts_u8_to_f32() {
        let out = convert_to_f32(&[0u8, 128, 255]);
        assert_eq!(out[0], -1.0);
        assert_eq!(out[1], 0.0);
        assert!((out[2] - (127.0 / 128.0)).abs() < 1e-6);
    }

    #[test]
    fn converts_u16_to_f32() {
        let out = convert_to_f32(&[0u16, 32_768, 65_535]);
        assert_eq!(out[0], -1.0);
        assert_eq!(out[1], 0.0);
        assert!((out[2] - (32_767.0 / 32_768.0)).abs() < 1e-6);
    }

    #[test]
    fn converts_i8_i32_f32_f64_to_f32() {
        assert_eq!(
            convert_to_f32(&[i8::MIN, 0, i8::MAX]),
            vec![-1.0, 0.0, 127.0 / 128.0] // dasp maps i8::MAX to 127/128
        );
        assert_eq!(convert_to_f32(&[i32::MIN, 0]), vec![-1.0, 0.0]);
        assert_eq!(convert_to_f32(&[0.5f32, -0.25]), vec![0.5, -0.25]);
        assert_eq!(convert_to_f32(&[0.5f64, -0.25]), vec![0.5, -0.25]);
    }

    #[test]
    fn converts_i24_and_u24_to_f32() {
        use dasp_sample::{I24, U24};
        // I24 range is -2^23 ..= 2^23 - 1; U24 is 0 ..= 2^24 - 1 with
        // equilibrium at 2^23 (dasp_sample::I24/U24::new returns Option).
        let i24 = [
            I24::new(-(1 << 23)).unwrap(),
            I24::new(0).unwrap(),
            I24::new((1 << 23) - 1).unwrap(),
        ];
        let out = convert_to_f32(&i24);
        assert_eq!(out[0], -1.0);
        assert_eq!(out[1], 0.0);
        assert!((out[2] - (8_388_607.0 / 8_388_608.0)).abs() < 1e-6);

        let u24 = [
            U24::new(0).unwrap(),
            U24::new(1 << 23).unwrap(),
            U24::new((1 << 24) - 1).unwrap(),
        ];
        let out = convert_to_f32(&u24);
        assert_eq!(out[0], -1.0);
        assert_eq!(out[1], 0.0);
        assert!((out[2] - (8_388_607.0 / 8_388_608.0)).abs() < 1e-6);
    }

    #[test]
    fn i16_max_round_trips_within_whisper_range() {
        // The full-scale sample must stay inside the closed -1.0..1.0 range
        // whisper expects; dasp maps i16::MAX slightly below 1.0.
        let out = convert_to_f32(&[i16::MAX]);
        assert!((-1.0..=1.0).contains(&out[0]));
    }

    // ---- Downmix fixtures (decision D4) ----

    #[test]
    fn mono_passthrough_unchanged() {
        let samples = vec![0.1, -0.2, 0.3];
        assert_eq!(downmix_to_mono(&samples, 1), samples);
    }

    #[test]
    fn stereo_downmix_averages_each_frame() {
        let samples = vec![0.0, 1.0, 0.5, 0.5, -1.0, -1.0];
        assert_eq!(downmix_to_mono(&samples, 2), vec![0.5, 0.5, -1.0]);
    }

    #[test]
    fn three_channel_downmix_averages_each_frame() {
        let samples = vec![1.0, 2.0, 3.0, 0.0, 0.0, 0.0];
        assert_eq!(downmix_to_mono(&samples, 3), vec![2.0, 0.0]);
    }

    #[test]
    fn downmix_drops_trailing_partial_frame() {
        // 5 interleaved stereo samples = 2 whole frames + 1 leftover sample.
        let samples = vec![0.0, 2.0, 4.0, 6.0, 99.0];
        assert_eq!(downmix_to_mono(&samples, 2), vec![1.0, 5.0]);
    }

    // ---- Resampling fixtures (decision D4) ----

    #[test]
    fn resample_equal_rate_returns_copy() {
        let samples = vec![0.1, -0.2, 0.3];
        assert_eq!(resample(&samples, 16_000, 16_000), samples);
    }

    #[test]
    fn resample_empty_input_returns_empty() {
        assert!(resample(&[], 48_000, 16_000).is_empty());
        assert!(resample(&[], 0, 16_000).is_empty());
    }

    #[test]
    fn resample_zero_rates_return_empty() {
        assert!(resample(&[0.1, 0.2], 0, 16_000).is_empty());
        assert!(resample(&[0.1, 0.2], 16_000, 0).is_empty());
    }

    #[test]
    fn resample_downsample_by_two() {
        // 4 samples at 4 kHz -> 2 samples at 2 kHz (positions 0 and 2).
        let samples = vec![0.0, 0.5, 1.0, 1.5];
        assert_eq!(resample(&samples, 4_000, 2_000), vec![0.0, 1.0]);
    }

    #[test]
    fn resample_upsample_by_two_interpolates() {
        // 2 samples at 2 kHz -> 4 samples at 4 kHz (positions 0, 0.5, 1, 1.5).
        let samples = vec![0.0, 1.0];
        assert_eq!(resample(&samples, 2_000, 4_000), vec![0.0, 0.5, 1.0, 1.0]);
    }

    #[test]
    fn resample_dc_signal_is_unchanged_at_any_rate() {
        // A constant signal must stay constant across a rate change; this
        // pins the interpolation math against drift.
        let samples = vec![0.25; 480];
        let out = resample(&samples, 48_000, 16_000);
        assert_eq!(out.len(), 160);
        assert!(out.iter().all(|&s| (s - 0.25).abs() < 1e-6));
    }

    #[test]
    fn resample_48k_to_whisper_16k_length_is_exact_third() {
        // The production path: 48 kHz capture down to 16 kHz. 1 second of
        // audio is 48_000 samples; the result must be exactly 16_000.
        let samples = vec![0.0; 48_000];
        let out = resample(&samples, 48_000, WHISPER_SAMPLE_RATE);
        assert_eq!(out.len(), 16_000);
    }

    #[test]
    fn resample_44100_to_16k_length_matches_duration() {
        let samples = vec![0.0; 44_100];
        let out = resample(&samples, 44_100, WHISPER_SAMPLE_RATE);
        assert_eq!(out.len(), 16_000);
    }

    #[test]
    fn dasp_conversion_helpers_agree_with_manual_math() {
        // Cross-check the dasp-based conversion against hand-computed values
        // for the formats a real ALSA device is most likely to provide.
        assert!(((-512_i16).to_sample::<f32>() - (-512.0 / 32_768.0)).abs() < 1e-6);
        assert_eq!(64u8.to_sample::<f32>(), -0.5);
    }
}
