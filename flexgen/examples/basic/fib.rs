//
// WARNING: This file has been auto-generated using flexgen (https://github.com/nu11ptr/flexgen).
// Any manual modifications to this file will be overwritten the next time this file is generated.
//

use std::error::Error as StdError;
use std::io::stdin;

/// This will run a compare between fib inputs and the outputs
/// ```
/// assert_eq!(fibonacci(10), 55);
/// assert_eq!(fibonacci(1), 1);
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
fn main() -> Result<(), Box<dyn StdError>> {
    println!("Enter a number:");
    let mut line = String::new();
    stdin().read_line(&mut line)?;
    let num: u64 = line.trim_end().parse()?;

    //
    // Calculate fibonacci for user input
    //
    let answer = fibonacci(num);
    println!("The number '{num}' in the fibonacci sequence is: {answer}");

    Ok(())
}
