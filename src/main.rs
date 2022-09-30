#![warn(clippy::pedantic, clippy::cargo)]

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::{fs, str};

use anyhow::{anyhow, Context};
use clap::{command, Arg, ArgAction};
use directories::ProjectDirs;
use gloss_word::{compile_results, get_response_text, get_section_vec, pandoc_primary, take_chunk};
use indicatif::{ProgressBar, ProgressStyle};
use rusqlite::Connection;
use scraper::{ElementRef, Selector};
use tempfile::NamedTempFile;

#[allow(clippy::too_many_lines)]
fn main() -> Result<(), anyhow::Error> {
    //
    // CLI SETUP
    //

    let matches = command!()
        .arg(
            Arg::new("clear-cache")
                .long("clear-cache")
                .help("Delete cache directory and its contents")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("etymology")
                .short('e')
                .long("etymology")
                .help("Search for etymology instead of definition")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("fetch-update")
                .short('f')
                .long("fetch-update")
                .help("Fetch new data; update cache if applicable")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("INPUT")
                .help("The word or phrase to look up")
                .required_unless_present("clear-cache"),
        )
        .get_matches();

    //
    // GLOBAL VARIABLES
    //

    // Do we have flags?
    let clear_cache = matches.get_flag("clear-cache");
    let etym_mode = matches.get_flag("etymology");
    let force_fetch = matches.get_flag("fetch-update");

    // Take input and lowercase it
    // Is this ok to unwrap?
    let mut desired_word = String::new();
    if !clear_cache {
        // I had trouble with type inference with the `get_one` method; hence this indirection
        let input_word: &String = matches.get_one("INPUT").unwrap();
        desired_word = input_word.clone().to_lowercase();
    }

    // What should be the path to the cache db? Is the db accessible?
    let mut db_path = PathBuf::new();
    let mut db_available = false;

    // Did we get a cache hit?
    let mut cache_hit = false;

    //
    // CACHE DIRECTORY
    //

    // Most operations here can fail silently; caching is optional
    if let Some(proj_dirs) = ProjectDirs::from("com", "theobeers", "gloss-word") {
        let cache_dir = proj_dirs.cache_dir();

        // If we have clear-cache flag, handle it and return
        if clear_cache && cache_dir.exists() {
            trash::delete(cache_dir)?;
            eprintln!("Cache directory deleted");
            return Ok(());
        } else if clear_cache {
            return Err(anyhow!("Cache directory not found"));
        }

        // If we don't have the cache dir yet, try to create it
        if !cache_dir.exists() {
            let _dir = fs::create_dir_all(cache_dir);
        }

        // Construct appropriate path for db
        db_path.push(cache_dir);
        db_path.push("entries.sqlite");
    }

    //
    // DB SETUP & CHECK FOR CACHED RESULTS
    //

    // Again, these operations can fail silently
    if let Ok(db_conn) = Connection::open(&db_path) {
        // Mark db available for later use
        db_available = true;

        // Create both tables, if they don't exist
        let _create_dic = db_conn.execute(
            "CREATE TABLE IF NOT EXISTS dictionary (
                    word        TEXT UNIQUE NOT NULL,
                    content     TEXT NOT NULL
                )",
            [],
        );

        let _create_etym = db_conn.execute(
            "CREATE TABLE IF NOT EXISTS etymology (
                    word        TEXT UNIQUE NOT NULL,
                    content     TEXT NOT NULL
                )",
            [],
        );

        // If we got a cache hit, handle it (usually print and return)
        if let Ok(entry) = query_db(&db_conn, &desired_word, etym_mode) {
            if force_fetch {
                cache_hit = true;
            } else {
                print!("{}", entry);
                return Ok(());
            }
        }
    }

    // Moving on...

    //
    // SCRAPING & CACHING
    //

    // Start a progress spinner; this could take a second
    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner} {msg}")
            .unwrap(),
    );
    pb.set_message("Fetching...");

    // Build the relevant URL
    let mut lookup_url: String;

    if etym_mode {
        lookup_url = "https://www.etymonline.com/word/".to_string();
        lookup_url += &desired_word.replace(' ', "%20");
    } else {
        lookup_url = "https://www.thefreedictionary.com/".to_string();
        lookup_url += &desired_word.replace(' ', "+");
    }

    // Make HTTP request and read response body into string
    let response_text = get_response_text(lookup_url)?;

    // Take desired chunk of response text (in definition mode)
    // In any case, parse what we have as an HTML tree
    let parsed_chunk = take_chunk(response_text);

    // Take specific selectors that we want
    let section_vec = get_section_vec(etym_mode, &parsed_chunk);

    // Check to see if we got any sections
    if !section_vec.is_empty() {
        // Compile results into string
        let results = compile_results(etym_mode, section_vec);

        // Call out to Pandoc
        let final_output = pandoc_primary(etym_mode, results)?;

        // Try to cache result; this can fail silently
        if db_available {
            let _update = update_cache(
                cache_hit,
                db_path,
                &desired_word,
                etym_mode,
                &final_output,
                force_fetch,
            );
        }

        // We still need to print results, of course
        // Also clear the spinner
        pb.finish_and_clear();
        print!("{}", final_output);
        return Ok(());
    }

    // Moving on...

    //
    // FALLBACK
    //

    // If we failed to get an etymology result, stop here
    if etym_mode {
        pb.finish_and_clear();
        return Err(anyhow!("Etymology not found"));
    }

    // In dictionary mode, we can check for a list of similar words
    let suggestions_selector = Selector::parse("ul.suggestions li").unwrap();
    let suggestions_vec: Vec<ElementRef> = parsed_chunk.select(&suggestions_selector).collect();

    // Again, see if we got anything
    if !suggestions_vec.is_empty() {
        // If so, collect results and push to string
        let mut results = String::new();

        for element in &suggestions_vec {
            results.push_str(&element.html());
        }

        // Call out to Pandoc
        let pandoc_output = pandoc_fallback(&results)?;

        // Print an explanatory message, then the results
        // Also clear the spinner
        pb.finish_and_clear();
        println!("Did you mean:\n");
        print!("{}", pandoc_output);
        return Ok(());
    }

    // If still no dice...
    pb.finish_and_clear();
    Err(anyhow!("Definition not found"))
}

// Function to call Pandoc in case of suggested alternate words
fn pandoc_fallback(results: &str) -> Result<String, anyhow::Error> {
    // Write results string into a tempfile to pass to Pandoc
    let mut pandoc_input = NamedTempFile::new().context("Failed to create tempfile")?;
    write!(pandoc_input, "{}", results).context("Failed to write to tempfile")?;

    let pandoc = Command::new("pandoc")
        .arg(pandoc_input.path())
        .arg("-f")
        .arg("html+smart-native_divs")
        .arg("-t")
        .arg("plain")
        .output()
        .context("Failed to execute Pandoc")?;

    let pandoc_output = str::from_utf8(&pandoc.stdout)
        .context("Failed to convert Pandoc output to string")?
        .to_string();

    Ok(pandoc_output)
}

// Function to query db for cached results
fn query_db(
    db_conn: &Connection,
    desired_word: &str,
    etym_mode: bool,
) -> Result<String, rusqlite::Error> {
    let mut query = String::new();

    // Construct query as appropriate
    if etym_mode {
        query.push_str("SELECT * FROM etymology WHERE word = '");
    } else {
        query.push_str("SELECT * FROM dictionary WHERE word = '");
    }

    query.push_str(desired_word);
    query.push('\'');

    let mut stmt = db_conn.prepare(&query)?;

    // We're looking for only one row, and only its definition/etymology column
    let entry_content: String = stmt.query_row([], |row| row.get(1))?;

    Ok(entry_content)
}

// Function to try to update cache with new results
fn update_cache(
    cache_hit: bool,
    db_path: PathBuf,
    desired_word: &str,
    etym_mode: bool,
    final_output: &str,
    force_fetch: bool,
) -> Result<(), rusqlite::Error> {
    // Yes, this means a second db connection; I don't think it's so bad
    let db_conn = Connection::open(db_path)?;

    // If we have force-fetch flag and got a cache hit, update
    if force_fetch && cache_hit {
        if etym_mode {
            db_conn.execute(
                "UPDATE etymology SET content = (?1) WHERE word = (?2)",
                [final_output, desired_word],
            )?;
        } else {
            db_conn.execute(
                "UPDATE dictionary SET content = (?1) WHERE word = (?2)",
                [final_output, desired_word],
            )?;
        }
    // Else insert
    } else if etym_mode {
        db_conn.execute(
            "INSERT INTO etymology (word, content) VALUES (?1, ?2)",
            [desired_word, final_output],
        )?;
    } else {
        db_conn.execute(
            "INSERT INTO dictionary (word, content) VALUES (?1, ?2)",
            [desired_word, final_output],
        )?;
    }

    Ok(())
}
