use bugspots::{run, Opts};
use structopt::StructOpt;

fn main() {
    let opts = Opts::from_args();
    match run(&opts) {
        Ok(()) => {}
        Err(e) => println!("error: {}", e),
    }
}
