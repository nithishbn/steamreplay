use anyhow::{Context, Result};
use scraper::{Html, Selector};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::thread;
use std::time::Duration;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage:");
        eprintln!("  {} scrape <steam_replay_url>", args[0]);
        eprintln!("  {} map-games [json_files...]", args[0]);
        eprintln!("  {} to-csv [json_files...]", args[0]);
        eprintln!("\nExamples:");
        eprintln!("  {} scrape https://store.steampowered.com/replay/76561198069815823/2024?l=english", args[0]);
        eprintln!("  {} map-games steam_replay_*.json", args[0]);
        eprintln!("  {} to-csv steam_replay_*.json", args[0]);
        std::process::exit(1);
    }

    let command = &args[1];

    match command.as_str() {
        "scrape" => {
            if args.len() < 3 {
                eprintln!("Error: Missing URL argument");
                eprintln!("Usage: {} scrape <steam_replay_url>", args[0]);
                std::process::exit(1);
            }
            scrape_replay(&args[2])?;
        }
        "map-games" => {
            if args.len() < 3 {
                eprintln!("Error: Missing JSON file argument(s)");
                eprintln!("Usage: {} map-games <json_files...>", args[0]);
                std::process::exit(1);
            }
            let json_files: Vec<String> = args[2..].to_vec();
            map_games_master(&json_files)?;
        }
        "to-csv" => {
            if args.len() < 3 {
                eprintln!("Error: Missing JSON file argument(s)");
                eprintln!("Usage: {} to-csv <json_files...>", args[0]);
                std::process::exit(1);
            }
            let json_files: Vec<String> = args[2..].to_vec();
            convert_to_csv(&json_files)?;
        }
        url if url.starts_with("http") => {
            // Backwards compatibility - treat first arg as URL
            scrape_replay(url)?;
        }
        _ => {
            eprintln!("Error: Unknown command '{}'", command);
            eprintln!("Valid commands: scrape, map-games, to-csv");
            std::process::exit(1);
        }
    }

    Ok(())
}

fn scrape_replay(url: &str) -> Result<()> {
    println!("Fetching Steam Replay from: {}", url);

    // Fetch the page
    let response = reqwest::blocking::get(url)
        .context("Failed to fetch the Steam Replay page")?;

    let html_content = response.text()
        .context("Failed to read response body")?;

    // Parse the HTML
    let document = Html::parse_document(&html_content);
    let selector = Selector::parse("#application_config")
        .expect("Failed to create selector");

    // Find the application_config div
    if let Some(element) = document.select(&selector).next() {
        println!("Found application_config div!");

        // Extract all data attributes
        let mut data_attributes = serde_json::Map::new();
        for (attr_name, attr_value) in element.value().attrs() {
            if attr_name.starts_with("data-") {
                // Try to parse as JSON first
                match serde_json::from_str::<serde_json::Value>(attr_value) {
                    Ok(json_value) => {
                        // Successfully parsed as JSON, store the parsed value
                        data_attributes.insert(attr_name.to_string(), json_value);
                        println!("  - {}: parsed as JSON", attr_name);
                    }
                    Err(_) => {
                        // Not valid JSON, store as string
                        data_attributes.insert(attr_name.to_string(), serde_json::Value::String(attr_value.to_string()));
                        println!("  - {}: {} chars (text)", attr_name, attr_value.len());
                    }
                }
            }
        }

        // Create output JSON
        let output = json!({
            "url": url,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "data": data_attributes
        });

        // Generate output filename
        let steam_id = extract_steam_id(url).unwrap_or("unknown");
        let year = extract_year(url).unwrap_or("unknown");
        let output_filename = format!("steam_replay_{}_{}.json", steam_id, year);

        // Write to file
        let output_json = serde_json::to_string_pretty(&output)
            .context("Failed to serialize JSON")?;

        fs::write(&output_filename, output_json)
            .context("Failed to write output file")?;

        println!("\nData saved to: {}", output_filename);
        println!("Found {} data attributes", data_attributes.len());
    } else {
        eprintln!("Error: Could not find div with id 'application_config'");
        std::process::exit(1);
    }

    Ok(())
}

fn map_games_master(json_files: &[String]) -> Result<()> {
    println!("Processing {} JSON file(s)...", json_files.len());

    // Collect all unique app IDs from all files
    let mut all_app_ids = HashSet::new();

    for json_file in json_files {
        println!("Reading: {}", json_file);

        let file_content = fs::read_to_string(json_file)
            .with_context(|| format!("Failed to read {}", json_file))?;

        let data: Value = serde_json::from_str(&file_content)
            .with_context(|| format!("Failed to parse {}", json_file))?;

        let app_ids = extract_app_ids(&data);
        println!("  Found {} app IDs", app_ids.len());
        all_app_ids.extend(app_ids);
    }

    println!("\nTotal unique app IDs across all files: {}", all_app_ids.len());

    // Fetch game names from Steam API
    let mut game_mapping: HashMap<String, String> = HashMap::new();
    let total = all_app_ids.len();

    for (index, app_id) in all_app_ids.iter().enumerate() {
        println!("[{}/{}] Fetching info for app ID: {}", index + 1, total, app_id);

        match fetch_game_name(app_id) {
            Ok(Some(name)) => {
                game_mapping.insert(app_id.clone(), name);
            }
            Ok(None) => {
                println!("  Warning: No data available for app ID {}", app_id);
            }
            Err(e) => {
                println!("  Error fetching app ID {}: {}", app_id, e);
            }
        }

        // Rate limiting - Steam API recommends spacing requests
        if index < total - 1 {
            thread::sleep(Duration::from_millis(1500));
        }
    }

    // Write master mapping as CSV
    let mapping_filename = "game_mapping_master.csv";
    let mut csv_content = String::from("app_id,game\n");

    let mut sorted_ids: Vec<_> = game_mapping.iter().collect();
    sorted_ids.sort_by_key(|&(id, _)| id);

    for (app_id, game_name) in sorted_ids {
        // Escape commas and quotes in game names
        let escaped_name = if game_name.contains(',') || game_name.contains('"') {
            format!("\"{}\"", game_name.replace('"', "\"\""))
        } else {
            game_name.clone()
        };
        csv_content.push_str(&format!("{},{}\n", app_id, escaped_name));
    }

    fs::write(mapping_filename, csv_content)
        .context("Failed to write mapping file")?;

    println!("\nMaster game mapping saved to: {}", mapping_filename);
    println!("Successfully mapped {} games", game_mapping.len());

    Ok(())
}

fn convert_to_csv(json_files: &[String]) -> Result<()> {
    println!("Converting {} JSON file(s) to CSV...", json_files.len());

    let mut csv_rows: Vec<(String, u64, String, String)> = Vec::new(); // (app_id, playtime_seconds, year, section)

    for json_file in json_files {
        println!("Processing: {}", json_file);

        let file_content = fs::read_to_string(json_file)
            .with_context(|| format!("Failed to read {}", json_file))?;

        let data: Value = serde_json::from_str(&file_content)
            .with_context(|| format!("Failed to parse {}", json_file))?;

        // Extract year from filename or data
        let year = extract_year_from_data(&data, json_file);
        println!("  Year: {}", year);

        // Extract playtime data with section tracking
        let playtime_data = extract_playtime_data(&data);
        println!("  Found {} playtime entries", playtime_data.len());

        for (app_id, playtime_seconds, section) in playtime_data {
            csv_rows.push((app_id, playtime_seconds, year.clone(), section));
        }
    }

    // Write CSV
    let csv_filename = "steam_replay_data.csv";
    let mut csv_content = String::from("app_id,playtime_in_seconds,year,month\n");

    csv_rows.sort_by(|a, b| {
        // Sort by year, then app_id, then section
        a.2.cmp(&b.2).then(a.0.cmp(&b.0)).then(a.3.cmp(&b.3))
    });

    for (app_id, playtime_seconds, year, section) in csv_rows {
        // Convert section to readable month name
        let month = convert_section_to_month(&section);

        csv_content.push_str(&format!("{},{},{},{}\n", app_id, playtime_seconds, year, month));
    }

    fs::write(csv_filename, csv_content)
        .context("Failed to write CSV file")?;

    println!("\nCSV data saved to: {}", csv_filename);

    Ok(())
}

fn extract_app_ids(value: &Value) -> HashSet<String> {
    let mut app_ids = HashSet::new();
    extract_app_ids_recursive(value, &mut app_ids);
    app_ids
}

fn extract_app_ids_recursive(value: &Value, app_ids: &mut HashSet<String>) {
    match value {
        Value::Object(map) => {
            // Check if this object has an "appid" field
            if let Some(Value::Number(app_id)) = map.get("appid") {
                if let Some(id) = app_id.as_u64() {
                    app_ids.insert(id.to_string());
                }
            }
            // Also check for "app_id" field
            if let Some(Value::Number(app_id)) = map.get("app_id") {
                if let Some(id) = app_id.as_u64() {
                    app_ids.insert(id.to_string());
                }
            }
            // Recursively search all values
            for val in map.values() {
                extract_app_ids_recursive(val, app_ids);
            }
        }
        Value::Array(arr) => {
            for val in arr {
                extract_app_ids_recursive(val, app_ids);
            }
        }
        _ => {}
    }
}

fn fetch_game_name(app_id: &str) -> Result<Option<String>> {
    let url = format!("https://store.steampowered.com/api/appdetails?appids={}", app_id);

    let response = reqwest::blocking::get(&url)
        .context("Failed to fetch game details")?;

    let data: Value = response.json()
        .context("Failed to parse response")?;

    // Steam API returns: { "appid": { "success": true/false, "data": {...} } }
    if let Some(app_data) = data.get(app_id) {
        if let Some(success) = app_data.get("success").and_then(|v| v.as_bool()) {
            if success {
                if let Some(name) = app_data
                    .get("data")
                    .and_then(|d| d.get("name"))
                    .and_then(|n| n.as_str())
                {
                    return Ok(Some(name.to_string()));
                }
            }
        }
    }

    Ok(None)
}

fn extract_year_from_data(data: &Value, filename: &str) -> String {
    // Try to extract year from URL in data
    if let Some(url) = data.get("url").and_then(|v| v.as_str()) {
        if let Some(year) = extract_year(url) {
            return year.to_string();
        }
    }

    // Try to extract from filename
    if let Some(year) = filename
        .split('_')
        .find(|part| part.len() == 4 && part.chars().all(|c| c.is_ascii_digit()))
    {
        return year.replace(".json", "");
    }

    "unknown".to_string()
}

fn extract_playtime_data(value: &Value) -> Vec<(String, u64, String)> {
    let mut playtime_data = Vec::new();
    extract_playtime_recursive(value, &mut playtime_data, &Vec::new());
    playtime_data
}

fn extract_playtime_recursive(value: &Value, playtime_data: &mut Vec<(String, u64, String)>, path: &Vec<String>) {
    match value {
        Value::Object(map) => {
            // Check if this object has both appid and relative_game_stats
            let has_appid = map.contains_key("appid");
            let has_relative_stats = map.contains_key("relative_game_stats");

            if has_appid && has_relative_stats {
                // Extract app_id
                let app_id = if let Some(Value::Number(id)) = map.get("appid") {
                    id.as_u64().map(|n| n.to_string())
                } else if let Some(Value::String(id)) = map.get("appid") {
                    Some(id.clone())
                } else {
                    None
                };

                // Extract playtime in seconds from relative_game_stats
                let playtime_seconds = map
                    .get("relative_game_stats")
                    .and_then(|stats| stats.get("total_playtime_seconds"))
                    .and_then(|v| v.as_u64());

                if let (Some(app_id), Some(playtime)) = (app_id, playtime_seconds) {
                    if playtime > 0 {
                        // Build section identifier from path
                        let section = if path.is_empty() {
                            "unknown".to_string()
                        } else {
                            path.join(".")
                        };
                        playtime_data.push((app_id, playtime, section));
                    }
                }
            }

            // Recursively search all values with updated path
            for (key, val) in map.iter() {
                // Build new path with meaningful keys
                let mut new_path = path.clone();

                // Only include certain keys in the path for better readability
                if key == "games" || key == "months" || key == "playtime_stats" {
                    new_path.push(key.clone());
                } else if key == "rtime_month" && path.last().map(|s| s.starts_with("month_")).unwrap_or(false) {
                    // For month objects, add the readable month to the path
                    if let Some(Value::Number(ts)) = map.get("rtime_month") {
                        if let Some(timestamp) = ts.as_i64() {
                            let month_str = format_month_from_timestamp(timestamp);
                            // Replace the last element (month_N) with readable month
                            if let Some(last) = new_path.last_mut() {
                                *last = month_str;
                            }
                        }
                    }
                }

                extract_playtime_recursive(val, playtime_data, &new_path);
            }
        }
        Value::Array(arr) => {
            for (index, val) in arr.iter().enumerate() {
                // For arrays under "months", track index as month number
                if path.last().map(|s| s.as_str()) == Some("months") {
                    let mut new_path = path.clone();
                    new_path.push(format!("month_{}", index));
                    extract_playtime_recursive(val, playtime_data, &new_path);
                } else {
                    extract_playtime_recursive(val, playtime_data, path);
                }
            }
        }
        _ => {}
    }
}

fn format_month_from_timestamp(timestamp: i64) -> String {
    // Convert Unix timestamp to a readable month format
    let datetime = chrono::DateTime::from_timestamp(timestamp, 0)
        .unwrap_or_else(|| chrono::DateTime::from_timestamp(0, 0).unwrap());
    datetime.format("%Y-%m").to_string()
}

fn convert_section_to_month(section: &str) -> String {
    // Convert section path to readable month name
    if section == "playtime_stats.games" {
        return "total".to_string();
    }

    // Extract month number from "playtime_stats.months.month_N"
    if let Some(month_part) = section.strip_prefix("playtime_stats.months.month_") {
        if let Ok(month_num) = month_part.parse::<usize>() {
            return get_month_name(month_num);
        }
    }

    // Default: return as is
    section.to_string()
}

fn get_month_name(month_index: usize) -> String {
    let months = [
        "January", "February", "March", "April", "May", "June",
        "July", "August", "September", "October", "November", "December"
    ];

    if month_index < 12 {
        months[month_index].to_string()
    } else {
        format!("month_{}", month_index)
    }
}

fn extract_steam_id(url: &str) -> Option<&str> {
    // Extract Steam ID from URL like: https://store.steampowered.com/replay/76561198069815823/2024
    url.split('/').nth_back(1)
}

fn extract_year(url: &str) -> Option<&str> {
    // Extract year from URL like: https://store.steampowered.com/replay/76561198069815823/2024
    let part = url.split('/').nth_back(0)?;
    // Remove query string if present
    part.split('?').next()
}
