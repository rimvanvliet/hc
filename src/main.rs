use std::collections::HashSet;
use std::io;
use std::io::Write;
use std::process;
use std::time::Instant;

use clap::{arg, command};
use regex::Regex;
use reqwest::{Client, Response, StatusCode};
use select::document::Document;
use select::predicate::Name;

// TODO allow for pages without /
// TODO improve error handling
// TODO add the page to the broken link

#[tokio::main]
async fn main() {
    let start = Instant::now();

    // prepare cli arg & flags
    let matches = command!()
        .arg_required_else_help(true)
        .arg(arg!([url] "Required url to operate on, including the protocol (so http or https)."))
        .arg(arg!(-d --debug "Turn debugging information on.")
            .required(false))
        .arg(arg!(-p --progress "Show a progress on-liner.")
            .required(false))
        .arg(arg!(-t --timer "Time execution.")
            .required(false))
        .get_matches();

    let base_url = matches.get_one::<String>("url").unwrap();
    let regex = Regex::new(r"^https?://[0-9A-Za-z.:]+$").unwrap();
    if !(regex.is_match(base_url)) {
        println!("{base_url} is not a valid url.");
        process::exit(1);
    }

    let debug = matches.get_flag("debug");
    let progress = matches.get_flag("progress");
    let timer = matches.get_flag("timer");

    if progress {
        print!("\rInternal pages checked: 0, Pages to go: 1, External links checked: 0                         ");
        io::stdout().flush().unwrap();
    }

    let client = Client::new();

    // initialise the progress; a page is an internal link, a link is an external link
    let mut checked_pages = HashSet::new();
    let mut checked_links = HashSet::new();
    // the check_result contains the result of an url check; Option<String>: None if ok, the link + error if not ok
    let mut check_results: HashSet<Option<String>> = HashSet::new();
    // initialise the pages that must still be checked with the root of the website
    // start with the base_url
    let mut to_be_checked_pages = HashSet::new();
    to_be_checked_pages.insert(base_url.to_string());

    while !to_be_checked_pages.is_empty() {
        // pop a (random) page to be checked
        let page_being_checked = to_be_checked_pages.iter().next().unwrap().clone();
        to_be_checked_pages.remove(&page_being_checked);

        if debug {
            println!("=============== Checking {page_being_checked}, remaining {} ====================", to_be_checked_pages.len());
            to_be_checked_pages.clone().into_iter().for_each(|s| print!("{s}, "));
            println!();
        }

        let hrefs = crawl(&client, &page_being_checked.to_string()).await;

        // determine the pages we did not yet see
        let new_pages = hrefs
            .clone()
            .into_iter()
            .map(|href| format_url(&href, base_url))
            .filter(|href| (href.starts_with('/') || href.starts_with(base_url))
                && href != &page_being_checked
                && !checked_pages.contains(href)
                && !to_be_checked_pages.contains(href))
            .collect::<HashSet<String>>();

        if debug {
            println!("new pages ({}); ", new_pages.len());
            new_pages.clone().into_iter().for_each(|s| print!("{s}, "));
            println!();
        }

        // determine the links we did not yet see
        let new_links = hrefs
            .clone()
            .into_iter()
            .map(|href| format_url(&href, base_url))
            .filter(|href| href.starts_with("http")
                && !href.starts_with(base_url)
                && !checked_links.contains(href))
            .collect::<HashSet<String>>();

        // concatenate new_pages and new_urls to check them in a batch
        let new_urls = [&Vec::from_iter(new_pages.clone())[..], &Vec::from_iter(new_links.clone())[..]].concat();

        for check_result in check_urls(&client, &page_being_checked, new_urls, debug).await {
            check_results.insert(check_result);
        }

        // insert the new_pages into the to_be_checked_pages
        new_pages
            .into_iter()
            .for_each(|item| {
                to_be_checked_pages.insert(item);
            });

        // and insert the new_links into the checked_links
        new_links
            .into_iter()
            .for_each(|item| {
                checked_links.insert(item);
            });

        // finally add the page_being_checked to the checked_pages
        checked_pages.insert(page_being_checked);

        if progress {
            print!("\rInternal pages checked: {}, Pages to go: {}, External links checked: {}                         ", checked_pages.len(), to_be_checked_pages.len(), checked_links.len());
            io::stdout().flush().unwrap();
        }
    }

    // print the results
    println!("\n--> de gevonden interne links zijn ({}): ", checked_pages.len());
    for int_link in checked_pages {
        println!("{int_link}");
    }
    println!("--> de gecheckte externe links zijn ({}): ", checked_links.len());
    for checked_url in checked_links {
        println!("{checked_url}");
    }
    let bad_urls = check_results
        .into_iter()
        .flatten().collect::<Vec<String>>();
    let nr_bad_urls = bad_urls.len();
    if nr_bad_urls == 0 {
        println!("--> er zijn GEEN gebroken urls.");
    } else {
        println!("--> LET OP: ER ZIJN GEBROKEN URLS ({}):", bad_urls.len());
        for bad_url in bad_urls {
            println!("{bad_url}");
        }
    }

    if timer {
        println!("Timer: {} seconden.", start.elapsed().as_secs());
    }

    // exit <> 0 if bad_urls exists
    if nr_bad_urls == 0 {
        process::exit(0);
    } else {
        process::exit(-1);
    }

    // strip a trailing '/' and/or the base_url from the url
    fn format_url(s: &str, prefix: &str) -> String {
        let mut tmp = s.to_string();
        if tmp.ends_with('/') {
            tmp.pop();
        }
        if tmp.starts_with('/') {
            format!("{}{}", prefix, tmp)
        } else {
            tmp
        }
    }

    // fetch all href's from the page, so both pages & links
    async fn crawl(client: &Client, url: &str) -> HashSet<String> {
        let mut links = HashSet::new();
        let body = client.get(url).send().await.unwrap().text().await.unwrap();
        let document = Document::from(body.as_str());
        for node in document.find(Name("a")) {
            let link = node.attr("href").unwrap_or("");
            links.insert(link.to_string());
        }
        links
    }

    // checks Vec<url> (pages or links) for HTTP status code between 200 and 299
    // returns None if ok, Some(link+error) if not ok
    async fn check_urls(client: &Client, page_being_checked: &str, urls: Vec<String>, debug: bool) -> HashSet<Option<String>> {
        let mut result = HashSet::new();

        let tasks = urls.into_iter().map(move |url| {
            fetch_url(client, url)
        });
        let responses: Vec<Result<Response, Box<dyn std::error::Error>>> = futures::future::join_all(tasks).await;

        for response in responses {
            match response {
                Ok(res) => {
                    if res.status() >= StatusCode::from_u16(200).unwrap()
                        && res.status() < StatusCode::from_u16(300).unwrap() {
                        if debug { println!("{}: success {:?}", res.url(), res.status()); }
                        result.insert(None);
                    } else {
                        if debug { println!("!!!!! ERROR {}: {:?}", res.url(), res.status()); }
                        result.insert(Some(format!("{} on {} gave status {:?}", res.url(), page_being_checked, res.status())));
                    }
                }
                Err(err) => {
                    if debug { println!("!!!!! ERROR {:?}", err); }
                    result.insert(Some(format!("error {:?}", err)));
                }
            }
        }
        result
    }
    // fetches a single url
    async fn fetch_url(client: &Client, url: String) -> Result<Response, Box<dyn std::error::Error>> {
        Ok(client
            .get(url)
            .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:108.0) Gecko/20100101 Firefox/108.0")
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8")
            .header("Accept-Language", "nl,en-US;q=0.7,en;q=0.3")
            .header("Accept-Encoding", "gzip, deflate, br")
            .header("DNT", "1")
            .header("Connection", "keep-alive")
            .header("Upgrade-Insecure-Requests", "1")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "none")
            .header("Sec-Fetch-User", "?1")
            .header("Pragma", "no-cache")
            .header("Cache-Control", "no-cache")
            .send()
            .await?)
    }
}

