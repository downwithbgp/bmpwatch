mod cli;
mod doctor;
mod error;
mod event;
mod input;
mod lint;
mod raw_bmp;
mod report;
mod state;

fn main() {
    cli::run();
}
