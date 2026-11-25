use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(err) = rseek::cli::run().await {
        eprintln!("rseek error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
