#![deny(warnings)]

use git2::{Commit, Oid, Repository};
use git2::{DiffOptions, Error};
use regex::Regex;
use std::collections::HashMap;
use std::str;
use structopt::StructOpt;
use time;
use time::{Timespec, Tm};

#[derive(Debug)]
struct Fix {
    message: String,
    date: Tm,
    files: Vec<String>,
}

#[derive(Debug)]
pub struct Spot {
    file: String,
    score: f64,
}

#[derive(StructOpt)]
pub struct Opts {
    #[structopt(name = "repo", long = "repo")]
    /// path of repository
    repo: String,
    #[structopt(name = "branch", short = "b", long = "branch")]
    /// branch to crawl
    branch: Option<String>,
    #[structopt(name = "depth", short = "d", long = "depth")]
    /// depth of log crawl (integer)
    depth: Option<usize>,
    #[structopt(name = "words", short = "w", long = "words")]
    /// bugfix indicator word list, ie: "fixes,closed"
    words: Option<String>,
    #[structopt(name = "regex", short = "r", long = "regex")]
    /// bugfix indicator regex, ie: "fix(es|ed)?" or "/fixes #(\d+)/i"
    regex: Option<String>,
    #[structopt(name = "display-timestamps", long = "display-timestamps")]
    /// show timestamps of each identified fix commit
    display_timestamps: Option<bool>,
}

struct Options {
    repo: String,
    branch: String,
    depth: Option<usize>,
    regex: Regex,
    display_timestamps: bool,
}

fn reg(args: &Opts) -> Result<Regex, regex::Error> {
    match reg_from_words(args) {
        Some(r) => Regex::new(r.as_str()),
        None => match &args.regex {
            Some(r) => Regex::new(r.as_str()),
            _ => Regex::new("(fix(es|ed)?|close([sd])?)"),
        },
    }
}

fn reg_from_words(args: &Opts) -> Option<String> {
    match &args.words {
        Some(w) => {
            let s: Vec<&str> = w.split(',').collect();
            Some(s.join("|"))
        }
        _ => None,
    }
}

fn scan(opts: &Options) -> Result<(Vec<Fix>, Vec<Spot>), Error> {
    let repo = Repository::open(&opts.repo)?;
    let obj = repo.revparse_single(opts.branch.as_str())?;

    let mut revwalk = repo.revwalk()?;
    revwalk.set_sorting(git2::Sort::TOPOLOGICAL)?;
    revwalk.push(obj.id())?;

    let f = |c: Result<Oid, Error>| {
        let id = match c {
            Ok(i) => i,
            Err(err) => panic!("{:?}", err),
        };
        let commit = match repo.find_commit(id) {
            Ok(c) => c,
            Err(err) => panic!("{:?}", err),
        };
        commit
    };
    let commits: Vec<Commit> = match opts.depth {
        Some(d) => {
            let t = revwalk.take(d);
            t.map(f).collect()
        }
        _ => revwalk.map(f).collect(),
    };

    let mut fixes: Vec<Fix> = Vec::new();
    for commit in commits {
        let lines = String::from_utf8_lossy(commit.message_bytes());
        let mut lines = lines.lines();
        let message = match lines.next() {
            Some(l) => String::from(l),
            _ => String::from(""),
        };

        if !opts.regex.is_match(message.as_str()) {
            continue;
        }

        let a = commit.parent(0)?;
        let a = a.tree()?;

        let b = commit.tree()?;
        let mut diffopts = DiffOptions::new();
        let diff = repo.diff_tree_to_tree(Some(&a), Some(&b), Some(&mut diffopts))?;
        let files = diff
            .deltas()
            .map(|s| {
                let path = s.old_file().path();
                match path {
                    Some(p) => match p.to_str() {
                        Some(s) => String::from(s),
                        _ => String::from(""),
                    },
                    _ => String::from(""),
                }
            })
            .filter(|p| p != "")
            .collect();

        fixes.push(Fix {
            message,
            date: time::at(Timespec {
                sec: commit.time().seconds(),
                nsec: 0,
            }),
            files,
        });
    }

    let mut hotspots: HashMap<String, f64> = HashMap::new();
    let current_time = time::now();
    let oldest_fix_date = &fixes.last().unwrap().date;
    for fix in &fixes {
        for file in &fix.files {
            let t = diff(&current_time, oldest_fix_date, &fix.date);
            let value = match hotspots.get(file.as_str()) {
                Some(t) => t,
                _ => &0.0,
            }
            .clone();
            hotspots.insert(String::from(file), t + value);
        }
    }

    let mut spots: Vec<Spot> = Vec::new();
    for (file, &n) in hotspots.iter() {
        spots.push(Spot {
            file: file.clone(),
            score: n,
        });
    }
    spots.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    Ok((fixes, spots))
}

fn diff(current_time: &Tm, oldest_fix_date: &Tm, fix_date: &Tm) -> f64 {
    // The timestamp used in the equation is normalized from 0 to 1, where
    // 0 is the earliest point in the code base, and 1 is now (where now is
    // when the algorithm was run). Note that the score changes over time
    // with this algorithm due to the moving normalization; it's not meant
    // to provide some objective score, only provide a means of comparison
    // between one file and another at any one point in time
    let t = 1.0
        - ((*current_time - *fix_date).num_seconds() as f64
            / (*current_time - *oldest_fix_date).num_seconds() as f64);
    1.0 / (1.0 + ((-12.0 * t) + 12.0).exp())
}

pub fn run(args: &Opts) -> Result<(), Error> {
    let options = Options {
        repo: args.repo.clone(),
        branch: args.branch.clone().unwrap_or("main".to_string()),
        depth: args.depth.clone(),
        regex: reg(&args).unwrap(),
        display_timestamps: args.display_timestamps.unwrap_or(false),
    };

    println!("Scanning {} repo", args.repo);

    let (fixes, spots) = scan(&options)?;

    println!(
        "\tFound {} bugfix commits, with {} hotspots:",
        fixes.len(),
        spots.len()
    );
    println!();

    println!("\tFixes:");
    for f in &fixes {
        let mut messages: Vec<String> = Vec::new();
        messages.push("\t\t-".to_string());
        if options.display_timestamps {
            messages.push(format!("{} ", f.date.rfc3339()))
        }
        messages.push(format!("{}", f.message));
        println!("{}", messages.join(" "));
    }

    println!();
    println!("\tHotspots:");
    for s in &spots {
        println!("\t\t{:.*} - {}", 4, s.score, s.file);
    }

    return Ok(());
}

#[cfg(test)]
mod tests {
    use crate::{diff, reg, run, Opts};
    use std::env;
    use std::ops::Sub;
    use time::Duration;

    #[test]
    fn test_init_repo() {
        let repo_path = env::var("TEST_REPO").unwrap();

        let opts = Opts {
            repo: repo_path,
            branch: Some("main".to_string()),
            depth: Some(200),
            words: None,
            regex: Some("\\[fix\\]".to_string()),
            display_timestamps: None,
        };

        let ret = run(&opts);

        assert_eq!(ret.unwrap(), ());
    }

    #[test]
    fn test_diff() {
        let current_time = time::now();
        let t = time::now();
        let fixes = time::now().sub(Duration::days(-10));
        let d = diff(&current_time, &t, &fixes);
        assert_eq!(d, 1.0);
    }

    #[test]
    fn test_reg() {
        let ret = reg(&Opts {
            repo: "".to_string(),
            branch: None,
            depth: None,
            words: Some("a,b,c".to_string()),
            regex: None,
            display_timestamps: None,
        });
        assert_eq!(ret.unwrap().as_str(), "a|b|c");
    }
}
