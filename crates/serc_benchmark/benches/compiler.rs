use std::fmt::Write;
use std::hint::black_box;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use serc_benchmark::criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main,
};
use serc_benchmark::{
    COMPILE_MICRO_WORKLOADS, COMPOSITE_WORKLOADS, LARGE_WORKLOADS, MARSHAL_MICRO_WORKLOADS,
};

fn compiler(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("compile");

    for workload in COMPILE_MICRO_WORKLOADS
        .iter()
        .chain(COMPOSITE_WORKLOADS)
        .chain(LARGE_WORKLOADS)
    {
        serc::compile(workload.source, workload.filename).unwrap();

        group.throughput(Throughput::Bytes(workload.source.len() as u64));
        group.bench_function(workload.id, |bencher| {
            bencher.iter_batched(
                || (),
                |()| serc::compile(black_box(workload.source), black_box(workload.filename)),
                BatchSize::LargeInput,
            );
        });
    }

    group.finish();
}

fn marshal(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("marshal");

    for workload in MARSHAL_MICRO_WORKLOADS
        .iter()
        .chain(COMPOSITE_WORKLOADS)
        .chain(LARGE_WORKLOADS)
    {
        group.throughput(Throughput::Bytes(workload.source.len() as u64));
        group.bench_function(workload.id, |bencher| {
            bencher.iter_batched(
                // A freshly compiled module has no serialization work left over from warmup
                // or earlier iterations. Cloning could preserve a future shared cache.
                || serc::compile(workload.source, workload.filename).unwrap(),
                |module| {
                    let output = black_box(&module).marshal();
                    // Keep cleanup outside timing when isolating the serialization phase.
                    (module, output)
                },
                BatchSize::LargeInput,
            );
        });
    }

    group.finish();
}

fn compile_pyc(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("compile_pyc");
    for workload in COMPILE_MICRO_WORKLOADS
        .iter()
        .chain(MARSHAL_MICRO_WORKLOADS)
        .chain(COMPOSITE_WORKLOADS)
        .chain(LARGE_WORKLOADS)
    {
        let source_size = u32::try_from(workload.source.len())
            .expect("benchmark source must fit in a PYC source-size field");
        group.throughput(Throughput::Bytes(workload.source.len() as u64));
        group.bench_function(workload.id, |bencher| {
            bencher.iter(|| {
                let module =
                    serc::compile(black_box(workload.source), black_box(workload.filename))
                        .unwrap();
                // Include encoding and both drops in this one-shot workflow. The PYC stays
                // in memory so disk I/O does not hide compiler or serializer changes.
                black_box(module.to_timestamp_pyc(0, source_size));
            });
        });
    }
    group.finish();
}

fn compile_pyc_optimized(criterion: &mut Criterion) {
    for (name, level) in [
        ("compile_pyc_O1", serc::OptimizationLevel::One),
        ("compile_pyc_O2", serc::OptimizationLevel::Two),
    ] {
        let options = serc::CompileOptions {
            optimization_level: level,
            ..serc::CompileOptions::default()
        };
        let mut group = criterion.benchmark_group(name);
        for workload in COMPILE_MICRO_WORKLOADS
            .iter()
            .chain(MARSHAL_MICRO_WORKLOADS)
            .chain(COMPOSITE_WORKLOADS)
            .chain(LARGE_WORKLOADS)
        {
            let source_size = u32::try_from(workload.source.len())
                .expect("benchmark source must fit in a PYC source-size field");
            group.throughput(Throughput::Bytes(workload.source.len() as u64));
            group.bench_function(workload.id, |bencher| {
                bencher.iter(|| {
                    let module = serc::compile_with_options(
                        black_box(workload.source),
                        black_box(workload.filename),
                        black_box(&options),
                    )
                    .unwrap();
                    black_box(module.to_timestamp_pyc(0, source_size));
                });
            });
        }
        group.finish();
    }
}

fn compiler_scaling(criterion: &mut Criterion) {
    let mut workloads = Vec::new();
    for count in [32, 128] {
        let mut exceptions = String::from("def work(x):\n");
        let mut comprehensions = String::from("def work(xs):\n");
        for index in 0..count {
            writeln!(
                exceptions,
                "    try:\n        x({index})\n    except ValueError:\n        x(-{})",
                index + 1,
            )
            .unwrap();
            writeln!(
                comprehensions,
                "    value_{index} = [y + {index} for y in xs if y]",
            )
            .unwrap();
        }
        exceptions.push_str("    return x\n");
        writeln!(comprehensions, "    return value_{}", count - 1).unwrap();
        workloads.push(("exceptions", count, exceptions));
        workloads.push(("comprehensions", count, comprehensions));
    }
    for count in [32, 128] {
        let mut async_with = String::from("async def work(manager, f):\n");
        let mut mapping = String::from("def work(x):\n    match x:\n        case {");
        for index in 0..count {
            writeln!(
                async_with,
                "    async with manager({index}) as value:\n        await f(value)",
            )
            .unwrap();
            write!(mapping, "'key_{index}': value_{index},").unwrap();
        }
        mapping.push_str("}:\n            return value_0\n");
        workloads.push(("async_with", count, async_with));
        workloads.push(("mapping_captures", count, mapping));
    }
    for count in [64, 256] {
        let mut membership = String::from("def work(value):\n    return value in {");
        for index in 0..count {
            write!(membership, "'member {index}',").unwrap();
        }
        membership.push_str("}\n");
        workloads.push(("string_set_membership", count, membership));
    }
    for count in [64, 256] {
        for (name, reverse) in [
            ("repeated_string_sets", false),
            ("reordered_string_sets", true),
        ] {
            let mut source = String::from("def work(value):\n    return (\n");
            for repetition in 0..8 {
                source.push_str("        value in {");
                for index in 0..count {
                    let index = if reverse && repetition % 2 != 0 {
                        count - index - 1
                    } else {
                        index
                    };
                    write!(source, "'member {index}',").unwrap();
                }
                source.push_str("},\n");
            }
            source.push_str("    )\n");
            workloads.push((name, count, source));
        }
    }

    for count in [64, 256] {
        let mut membership = String::from("def work(value):\n    return value in {");
        for index in 0..count {
            write!(membership, "{index},").unwrap();
        }
        membership.push_str("}\n");
        workloads.push(("integer_set_membership", count, membership));
    }

    for (name, python_version) in [
        ("compile_pyc_scaling_3_12", serc::PythonVersion::Py312),
        ("compile_pyc_scaling_3_13", serc::PythonVersion::Py313),
        ("compile_pyc_scaling_3_14", serc::PythonVersion::Py314),
        ("compile_pyc_scaling_3_15", serc::PythonVersion::Py315),
    ] {
        let options = serc::CompileOptions {
            python_version,
            ..serc::CompileOptions::default()
        };
        let mut group = criterion.benchmark_group(name);
        for (shape, count, source) in &workloads {
            let source_size = u32::try_from(source.len())
                .expect("benchmark source must fit in a PYC source-size field");
            group.throughput(Throughput::Bytes(source.len() as u64));
            group.bench_function(BenchmarkId::new(*shape, count), |bencher| {
                bencher.iter(|| {
                    let module = serc::compile_with_options(
                        black_box(source),
                        black_box("benchmark/scaling.py"),
                        black_box(&options),
                    )
                    .unwrap();
                    black_box(module.to_timestamp_pyc(0, source_size));
                });
            });
        }
        group.finish();
    }
}

criterion_group!(
    benches,
    compiler,
    marshal,
    compile_pyc,
    compile_pyc_optimized,
    compiler_scaling
);
criterion_main!(benches);
