//
// WARNING: This file has been auto-generated using flexgen (https://github.com/nu11ptr/flexgen).
// Any manual modifications to this file will be overwritten the next time this file is generated.
//

/// This will run a compare between fib inputs and the outputs
/// ```
/// assert_eq!(fibonacci(10), 55);
/// assert_eq!(fibonacci(1), 1);
/// println!("Fib: {}", fibonacci(12));
/// ```
#[inline]
fn fibonacci(n: u64) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        n => fibonacci(n - 1) + fibonacci(n - 2),
    }
}

/// This is the main function
fn main() {
    //
    // Calculate fibonacci for the number 42
    //
    let answer = fibonacci(42);
    println!("{answer}");
}
