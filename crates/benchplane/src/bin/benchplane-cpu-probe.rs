// SPDX-License-Identifier: Apache-2.0

use benchplane_schema::{
    MeasurementPhase, MeasurementRecord, CPU_PROBE_GENERATOR_VERSION, MAX_CPU_PROBE_OUTPUT_TOKENS,
    MAX_CPU_PROBE_RECORDS, MAX_CPU_PROBE_TOTAL_WORK_UNITS, MAX_CPU_PROBE_WORK_UNITS_PER_TOKEN,
};
use clap::Parser;
use std::{
    hint::black_box,
    io::{self, Write},
    process::ExitCode,
    time::{Duration, Instant},
};

#[derive(Debug, Parser)]
#[command(name = "benchplane-cpu-probe", hide = true)]
struct Args {
    #[arg(long)]
    requests: u32,
    #[arg(long)]
    warmup_runs: u32,
    #[arg(long)]
    repetitions: u32,
    #[arg(long)]
    output_tokens: u32,
    #[arg(long)]
    work_units_per_token: u32,
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<(), String> {
    validate(&args)?;
    let stdout = io::stdout();
    let mut output = io::BufWriter::new(stdout.lock());
    for index in 1..=args.warmup_runs {
        emit_repetition(&mut output, &args, MeasurementPhase::Warmup, index)?;
    }
    for index in 1..=args.repetitions {
        emit_repetition(&mut output, &args, MeasurementPhase::Measured, index)?;
    }
    Ok(())
}

fn validate(args: &Args) -> Result<(), String> {
    if args.requests == 0 || args.repetitions == 0 {
        return Err("requests and repetitions must be positive".to_owned());
    }
    if !(1..=MAX_CPU_PROBE_OUTPUT_TOKENS).contains(&args.output_tokens)
        || !(1..=MAX_CPU_PROBE_WORK_UNITS_PER_TOKEN).contains(&args.work_units_per_token)
    {
        return Err("CPU probe controls are outside their public bounds".to_owned());
    }
    let records = args
        .warmup_runs
        .checked_add(args.repetitions)
        .ok_or_else(|| "CPU probe record count overflowed".to_owned())?;
    if records > MAX_CPU_PROBE_RECORDS {
        return Err(format!(
            "CPU probe record count exceeds its {MAX_CPU_PROBE_RECORDS} record public bound"
        ));
    }
    let total = u64::from(records)
        .checked_mul(u64::from(args.requests))
        .and_then(|value| value.checked_mul(u64::from(args.output_tokens)))
        .and_then(|value| value.checked_mul(u64::from(args.work_units_per_token)))
        .ok_or_else(|| "CPU probe total work overflowed".to_owned())?;
    if total > MAX_CPU_PROBE_TOTAL_WORK_UNITS {
        return Err("CPU probe total work exceeds its public bound".to_owned());
    }
    Ok(())
}

fn emit_repetition(
    output: &mut impl Write,
    args: &Args,
    phase: MeasurementPhase,
    repetition_index: u32,
) -> Result<(), String> {
    let repetition_start = Instant::now();
    let mut latency_total = Duration::ZERO;
    let mut first_output_total = Duration::ZERO;
    let mut checksum = 0_u64;

    for request_index in 0..args.requests {
        let request_start = Instant::now();
        let mut first_output = None;
        let mut state = checksum
            ^ u64::from(request_index).wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ u64::from(repetition_index);
        for token_index in 0..args.output_tokens {
            state = token_work(state ^ u64::from(token_index), args.work_units_per_token);
            black_box(state);
            if first_output.is_none() {
                first_output = Some(request_start.elapsed());
            }
        }
        checksum ^= black_box(state);
        first_output_total += first_output.expect("positive output token count");
        latency_total += request_start.elapsed();
    }
    black_box(checksum);

    let repetition_elapsed = repetition_start.elapsed();
    let request_count = u64::from(args.requests);
    let latency_micros = ceil_micros(latency_total) / request_count;
    let ttft_micros = (ceil_micros(first_output_total) / request_count).min(latency_micros);
    let wall_micros = u128::from(ceil_micros(repetition_elapsed));
    let throughput: u64 = (u128::from(args.requests) * 1_000 * 1_000_000 / wall_micros)
        .try_into()
        .map_err(|_| "CPU probe throughput overflowed".to_owned())?;
    let record = MeasurementRecord {
        generator: CPU_PROBE_GENERATOR_VERSION.to_owned(),
        attempt_number: 1,
        phase,
        repetition_index,
        sample_index: 1,
        latency_micros: latency_micros.max(1),
        time_to_first_token_micros: ttft_micros.max(1),
        throughput_milli_requests_per_second: throughput.max(1),
        successful_requests: args.requests,
        failed_requests: 0,
    };
    serde_json::to_writer(&mut *output, &record)
        .map_err(|error| format!("could not serialize measurement: {error}"))?;
    output
        .write_all(b"\n")
        .and_then(|()| output.flush())
        .map_err(|error| format!("could not write measurement: {error}"))
}

fn token_work(mut state: u64, work_units: u32) -> u64 {
    for index in 0..work_units {
        state = state
            .wrapping_add(u64::from(index) ^ 0xa076_1d64_78bd_642f)
            .rotate_left((state as u32 & 31) + 1)
            .wrapping_mul(0xe703_7ed1_a0b4_28db);
        state ^= state >> 23;
        black_box(state);
    }
    state
}

fn ceil_micros(duration: Duration) -> u64 {
    duration
        .as_nanos()
        .div_ceil(1_000)
        .max(1)
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_work_is_data_dependent() {
        assert_ne!(token_work(1, 32), token_work(2, 32));
        assert_ne!(token_work(1, 32), token_work(1, 33));
    }

    #[test]
    fn emitted_measurements_have_observed_semantics() {
        let args = Args {
            requests: 2,
            warmup_runs: 1,
            repetitions: 2,
            output_tokens: 4,
            work_units_per_token: 64,
        };
        let mut output = Vec::new();
        emit_repetition(&mut output, &args, MeasurementPhase::Measured, 1).unwrap();
        let record: MeasurementRecord = serde_json::from_slice(&output).unwrap();
        assert_eq!(record.generator, CPU_PROBE_GENERATOR_VERSION);
        assert_eq!(record.successful_requests, 2);
        assert_eq!(record.failed_requests, 0);
        assert!(record.latency_micros > 0);
        assert!(record.time_to_first_token_micros > 0);
        assert!(record.time_to_first_token_micros <= record.latency_micros);
        assert!(record.throughput_milli_requests_per_second > 0);
    }

    fn minimal_args(records: u32) -> Args {
        Args {
            requests: 1,
            warmup_runs: 0,
            repetitions: records,
            output_tokens: 1,
            work_units_per_token: 1,
        }
    }

    #[test]
    fn direct_helper_enforces_the_public_record_limit() {
        assert_eq!(validate(&minimal_args(MAX_CPU_PROBE_RECORDS)), Ok(()));
        assert!(validate(&minimal_args(MAX_CPU_PROBE_RECORDS + 1))
            .expect_err("helper must reject a record count above the public maximum")
            .contains("record count exceeds"));
    }
}
