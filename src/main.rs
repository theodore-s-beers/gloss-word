#![warn(clippy::pedantic, clippy::nursery, clippy::cargo)]

use anyhow::anyhow;
use clap::{Arg, ArgAction, command};
use directories::ProjectDirs;
use gloss_word::{
    compile_results, get_response_text, get_section_vec, pandoc_fallback, pandoc_primary,
    take_chunk,
};
use indicatif::{ProgressBar, ProgressStyle};
use rusqlite::Connection;
use scraper::{ElementRef, Selector};

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
    // "GLOBAL" VARIABLES
    //

    // Do we have flags?
    let clear_cache = matches.get_flag("clear-cache");
    let etym_mode = matches.get_flag("etymology");
    let force_fetch = matches.get_flag("fetch-update");

    // Take input and lowercase it
    let desired_word = if clear_cache {
        String::new() // Placeholder; we'll return soon, anyway
    } else {
        let input_word: &String = matches.get_one("INPUT").unwrap(); // Should be ok
        input_word.to_lowercase()
    };

    let mut db_conn = None;

    //
    // CACHE DIRECTORY
    //

    // Most operations here can fail silently; caching is optional
    if let Some(proj_dirs) = ProjectDirs::from("com", "theobeers", "gloss-word") {
        let cache_dir = proj_dirs.cache_dir();

        // If we have clear-cache flag, handle it and return
        if clear_cache {
            if !cache_dir.exists() {
                return Err(anyhow!("Cache directory not found"));
            }

            trash::delete(cache_dir)?;
            eprintln!("Cache directory deleted");
            return Ok(());
        }

        // If we don't have the cache dir yet, try to create it
        if !cache_dir.exists() {
            let _dir = std::fs::create_dir_all(cache_dir);
        }

        db_conn = open_cache(&cache_dir.join("entries.sqlite")).ok();
    }

    //
    // DB SETUP & CHECK FOR CACHED RESULTS
    //

    // Again, these operations can fail silently
    if let Some(db_conn) = &db_conn {
        // If we got a cache hit, handle it (usually print and return)
        if let Ok(entry) = query_db(db_conn, &desired_word, etym_mode)
            && !force_fetch
        {
            print!("{entry}");
            return Ok(());
        }
    }

    // Moving on...

    //
    // SCRAPING & CACHING
    //

    // Start a progress spinner; this could take a second
    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(core::time::Duration::from_millis(80));
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
        lookup_url = "https://www.etymonline.com/word/".to_owned();
        lookup_url += &desired_word.replace(' ', "%20");
    } else {
        lookup_url = "https://www.thefreedictionary.com/".to_owned();
        lookup_url += &desired_word.replace(' ', "+");
    }

    // Make HTTP request and read response body into string
    let response_text = get_response_text(&lookup_url)?;

    // Take desired chunk of response text (in definition mode)
    // In any case, parse what we have as an HTML tree
    let parsed_chunk = take_chunk(&response_text);

    // Take specific selectors that we want
    let section_vec = get_section_vec(etym_mode, &parsed_chunk);

    // If we got one or more sections...
    if !section_vec.is_empty() {
        // Compile results into string
        let results = compile_results(etym_mode, section_vec);

        // Call out to Pandoc
        let final_output = pandoc_primary(&results, etym_mode)?;

        // Try to cache result; this can fail silently
        if let Some(db_conn) = &db_conn {
            let _update = update_cache(db_conn, &desired_word, etym_mode, &final_output);
        }

        // We still need to print results, of course (after clearing the spinner)
        pb.finish_and_clear();
        print!("{final_output}");
        return Ok(());
    }

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

    // If we got something...
    if !suggestions_vec.is_empty() {
        let mut results = String::new();

        for element in &suggestions_vec {
            results.push_str(&element.html());
        }

        // Call out to Pandoc
        let pandoc_output = pandoc_fallback(&results)?;

        // Print an explanatory message, then the results (after clearing the spinner)
        pb.finish_and_clear();
        println!("Did you mean:\n");
        print!("{pandoc_output}");
        return Ok(());
    }

    // If still no dice...
    pb.finish_and_clear();
    Err(anyhow!("Definition not found"))
}

fn open_cache(path: &std::path::Path) -> Result<Connection, rusqlite::Error> {
    let db_conn = Connection::open(path)?;
    db_conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS dictionary (
            word        TEXT UNIQUE NOT NULL,
            content     TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS etymology (
            word        TEXT UNIQUE NOT NULL,
            content     TEXT NOT NULL
        );",
    )?;
    Ok(db_conn)
}

const fn cache_table(etym_mode: bool) -> &'static str {
    if etym_mode { "etymology" } else { "dictionary" }
}

// Function to query db for cached results
fn query_db(
    db_conn: &Connection,
    desired_word: &str,
    etym_mode: bool,
) -> Result<String, rusqlite::Error> {
    let query = format!(
        "SELECT content FROM {} WHERE word = ?1",
        cache_table(etym_mode)
    );
    let mut stmt = db_conn.prepare(&query)?;

    // We're looking for only one row, and only its definition/etymology column
    let entry_content: String = stmt.query_row([desired_word], |row| row.get(0))?;

    Ok(entry_content)
}

// Function to try to update cache with new results
fn update_cache(
    db_conn: &Connection,
    desired_word: &str,
    etym_mode: bool,
    final_output: &str,
) -> Result<(), rusqlite::Error> {
    let query = format!(
        "INSERT INTO {} (word, content) VALUES (?1, ?2)
         ON CONFLICT(word) DO UPDATE SET content = excluded.content",
        cache_table(etym_mode)
    );
    db_conn.execute(&query, [desired_word, final_output])?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cache() -> Connection {
        open_cache(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn cache_round_trip_handles_apostrophes() {
        let db_conn = test_cache();

        update_cache(&db_conn, "o'clock", false, "a time").unwrap();

        assert_eq!(
            query_db(&db_conn, "o'clock", false).unwrap(),
            "a time".to_owned()
        );
    }

    #[test]
    fn cache_update_replaces_existing_content_in_the_selected_table() {
        let db_conn = test_cache();

        update_cache(&db_conn, "forest", true, "old").unwrap();
        update_cache(&db_conn, "forest", true, "new").unwrap();

        assert_eq!(
            query_db(&db_conn, "forest", true).unwrap(),
            "new".to_owned()
        );
        assert!(query_db(&db_conn, "forest", false).is_err());
    }
}
