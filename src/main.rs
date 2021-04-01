use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::{env, fs};

use isahc::prelude::*;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};

fn main() -> Result<(), isahc::Error> {
    // Collect args
    let args: Vec<String> = env::args().collect();

    // If a word wasn't given, panic
    if args.len() < 2 {
        panic!("No word was provided");
    };

    // First real arg is the desired word
    let desired_word = &args[1];

    // See if we can get the home directory
    let home_dir = home::home_dir();

    // Is there a path that we should check for a cached result?
    // False by default
    let mut maybe_file_path = false;

    // Set up a string for a possible path
    let mut notional_file_path = String::new();

    // We will also need to check for the cache directory
    let mut cache_path = String::new();

    // Let's make a global variable for cache directory availability
    // False until further notice
    let mut cache_path_exists = false;

    // If we did get a home directory, assemble the notional file path
    if let Some(path) = &home_dir {
        let home_dir_str = path.to_string_lossy();

        // println!("Home dir path: {}", home_dir_str);

        notional_file_path.push_str(&home_dir_str);
        notional_file_path.push_str("/.ahd-scrape/cache/");
        notional_file_path.push_str(desired_word);
        notional_file_path.push_str(".txt");

        // println!("Desired file path: {}", notional_file_path);

        // Also put together the cache directory path
        cache_path.push_str(&home_dir_str);
        cache_path.push_str("/.ahd-scrape/cache");

        // println!("Cache dir path: {}", cache_path);

        // Test for availability of the cache path
        // Set the global boolean accordingly
        // I had a hard time appeasing the compiler here
        let test_cache_path = Path::new(&cache_path).exists();
        if test_cache_path {
            cache_path_exists = true;
        }

        // println!("First check for cache dir: {}", cache_path_exists);

        // If it isn't there, try creating it
        if !cache_path_exists {
            let try_create_cache = fs::create_dir_all(&cache_path);
            if let Ok(_) = try_create_cache {
                cache_path_exists = true;
            }
        }

        // println!("Second check for cache dir: {}", cache_path_exists);

        // Now, if we have the cache directory, set maybe_file_path to true
        if cache_path_exists {
            maybe_file_path = true;
        }
    }

    // Does the notional file path exist? False by default
    let mut file_path_exists = false;

    // If all is well so far, check the file path
    // If it's there (and can be accessed), this will evaluate to true
    if maybe_file_path {
        file_path_exists = Path::new(&notional_file_path).exists();
    }

    // println!("Check for file path: {}", file_path_exists);

    // Now we try to read the file at the given path
    // All errors are ignored; the program would just move on
    if file_path_exists {
        let try_open = File::open(&notional_file_path);

        if let Ok(mut file) = try_open {
            let mut contents = String::new();
            let try_read = file.read_to_string(&mut contents);

            if let Ok(_) = try_read {
                // Success? Print file contents (to be piped) and return
                print!("{}", contents);

                return Ok(());
            }
        }
    }

    // If we're moving on, it means we didn't get a cached result
    // This is, of course, the default case

    // Assemble URL
    let mut dictionary_url = "https://www.thefreedictionary.com/".to_string();
    dictionary_url += desired_word;

    // Make the request
    let mut response = isahc::get(dictionary_url)?;

    // Get the document text
    let response_text = response.text()?;

    // Set up a regex to split the document
    let re = Regex::new(r#"<div id="Thesaurus">"#).unwrap();

    // Split document into two chunks
    // Otherwise we could blow a bunch of time parsing the whole thing
    let chunks: Vec<&str> = re.split(&response_text).collect();

    // Parse the first chunk, which is the one we want
    let parsed_chunk = Html::parse_fragment(chunks[0]);

    // Set up a selector for the relevant section
    let section_selector = Selector::parse(r#"div#Definition section[data-src="hm"]"#).unwrap();

    // Run the select iterator and collect the result in a vec
    // This should yield either one item, or nothing
    let section_vec: Vec<ElementRef> = parsed_chunk.select(&section_selector).collect();

    // Set up a few more selectors, just for the desired elements
    let element_selectors = Selector::parse("div.pseg, h2, hr.hmsep").unwrap();

    // Check to see if we got the section we wanted
    if section_vec.len() > 0 {
        // If so, we'll iterate over the section and grab elements
        // This maintains document order, unlike pup, which drove me nuts

        // Set up a string to hold results
        let mut results = String::new();

        for element in section_vec[0].select(&element_selectors) {
            results.push_str(&element.html());
        }

        // If the cache path proved available earlier, try to create a file
        if cache_path_exists {
            let new_file = File::create(&notional_file_path);

            // If we have the new file, write the results into it
            // Again, errors are just ignored
            if let Ok(mut file) = new_file {
                let _ = file.write_all(results.as_bytes());
            };
        }

        // We still need to print results, since they get piped along
        print!("{}", results);
    } else {
        // Otherwise let's check for a list of similar words
        let suggestions_selector = Selector::parse(r#"ul.suggestions li"#).unwrap();
        let suggestions_vec: Vec<ElementRef> = parsed_chunk.select(&suggestions_selector).collect();

        // Again, see if we got anything
        if suggestions_vec.len() > 0 {
            // If so, start by printing a brief explanatory message
            print!("Did you mean:");

            // Iterate over the vec of elements and print them all
            for element in suggestions_vec.iter() {
                print!("{}", element.html());
            }
        } else {
            // If still no dice, panic
            panic!("Definition not found");
        }
    }

    Ok(())
}
