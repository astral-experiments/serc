use crate::common;

#[test]
fn matches_cpython_nonfinite_complex_multiplication() {
    let operands = [
        "1e309 + (1e309j - 1e309j)",
        "-1e309 + (1e309j - 1e309j)",
        "(1e309 - 1e309) + 1e309j",
        "(1e309 - 1e309) - 1e309j",
    ];
    let multipliers = ["1 + 1j", "1 - 1j", "-1 + 1j", "-1 - 1j"];
    for operand in operands {
        for multiplier in multipliers {
            for (left, right) in [(operand, multiplier), (multiplier, operand)] {
                let source = format!("def product():\n    return ({left}) * ({right})\n");
                common::assert_matches_cpython_marshal(&source, "complex_multiplication.py");
            }
        }
    }

    for expression in [
        "(1e309 + (1e309j - 1e309j)) * 1.0",
        "1.0 * (1e309 + (1e309j - 1e309j))",
        "(1e309 + 1e309j) * (1e309 + 1e309j)",
        "(1e300 + 1e300j) * (1e300 - 1e300j)",
        "(1e309 + 1j) * (1 + 1j)",
        "(1e309 + 1j) * 0j",
        "(1e309 - 1e309) * 1j",
        "(1 + 2j) * (3 + 4j)",
    ] {
        let source = format!("def product():\n    return {expression}\n");
        common::assert_matches_cpython_marshal(&source, "complex_multiplication_control.py");
    }
}

#[test]
fn executes_folded_complex_multiplication_like_runtime() {
    common::assert_executes(
        r"
import math

infinity = float('inf')
nan = float('nan')
values = [
    ((infinity + complex(0, nan), 1 + 1j),
     (1e309 + (1e309j - 1e309j)) * (1 + 1j)),
    ((1 + 1j, infinity + complex(0, nan)),
     (1 + 1j) * (1e309 + (1e309j - 1e309j))),
    ((complex(nan, infinity), 1 + 1j),
     ((1e309 - 1e309) + 1e309j) * (1 + 1j)),
    ((complex(nan, -infinity), 1 - 1j),
     ((1e309 - 1e309) - 1e309j) * (1 - 1j)),
]
for (left, right), folded in values:
    runtime = left * right
    print(folded == runtime, math.isinf(folded.real), math.isinf(folded.imag))
",
        "True True True\nTrue True True\nTrue True True\nTrue True True\n",
    );
}
