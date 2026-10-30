//! Terminal output: tables, progress bars and messages.

use std::path::Path;
use std::time::Duration;

use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use voxear_launcher::Progress;

pub fn error(error: &anyhow::Error) {
    eprintln!("{} {error:#}", style("error:").red().bold());
}

pub fn warn(message: &str) {
    eprintln!("{} {message}", style("warning:").yellow().bold());
}

pub fn done(message: &str) {
    println!("{} {message}", style("✓").green().bold());
}

pub fn hint(message: &str) {
    println!("  {}", style(message).dim());
}

/// A path with the home folder shortened to `~`.
pub fn pretty(path: &Path) -> String {
    if let Some(home) = std::env::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
        && !cfg!(windows)
    {
        return Path::new("~").join(rest).display().to_string();
    }
    path.display().to_string()
}

/// Prints rows as aligned columns. The last column is never padded.
pub fn table(rows: &[Vec<String>]) {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|column| {
            rows.iter()
                .filter_map(|row| row.get(column))
                .map(|cell| console::measure_text_width(cell))
                .max()
                .unwrap_or(0)
        })
        .collect();
    for row in rows {
        let mut line = String::from("  ");
        for (column, cell) in row.iter().enumerate() {
            line.push_str(cell);
            if column + 1 < row.len() {
                let pad = widths[column] - console::measure_text_width(cell);
                line.push_str(&" ".repeat(pad + 2));
            }
        }
        println!("{}", line.trim_end());
    }
}

/// A progress bar for installs. It draws nothing when output isn't a terminal.
pub struct InstallBar {
    bar: ProgressBar,
    phase: &'static str,
}

impl InstallBar {
    pub fn new() -> Self {
        let bar = ProgressBar::new(0);
        bar.enable_steady_tick(Duration::from_millis(100));
        Self { bar, phase: "" }
    }

    pub fn update(&mut self, progress: Progress) {
        match progress {
            Progress::Downloading { done, total } => {
                self.enter("Downloading", "{msg:12} {bar:32.cyan/blue} {bytes:>10}/{total_bytes} {bytes_per_sec:>12}");
                self.bar.set_length(total);
                self.bar.set_position(done);
            }
            Progress::Verifying => self.enter("Verifying", "{msg:12} {spinner}"),
            Progress::Unpacking { done, total } => {
                self.enter("Unpacking", "{msg:12} {bar:32.cyan/blue} {percent:>3}%");
                self.bar.set_length(total);
                self.bar.set_position(done);
            }
            Progress::Finishing => self.enter("Finishing", "{msg:12} {spinner}"),
        }
    }

    pub fn finish(&self) {
        self.bar.finish_and_clear();
    }

    fn enter(&mut self, phase: &'static str, template: &str) {
        if self.phase == phase {
            return;
        }
        self.phase = phase;
        let style = ProgressStyle::with_template(template).expect("the template is valid").progress_chars("━╸ ");
        self.bar.set_style(style);
        self.bar.set_message(phase);
        self.bar.set_position(0);
    }
}

impl Drop for InstallBar {
    fn drop(&mut self) {
        self.bar.finish_and_clear();
    }
}
