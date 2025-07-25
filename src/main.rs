#[macro_export]
macro_rules! log{
    ($($args:tt)*) => {
        let log_message = format_args!($($args)*);
        println!("[NES] {}", log_message);
    };
}

fn main() {
    log!("Starting NES emu...");
}
