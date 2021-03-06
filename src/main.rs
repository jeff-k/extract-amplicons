extern crate bio;
extern crate clap;
extern crate csv;
extern crate flate2;
extern crate serde;

use clap::{App, Arg};

use bio::alignment::pairwise::*;
use bio::alignment::AlignmentOperation::Match;
use bio::io::fastq::{Reader, Record};

use flate2::bufread::MultiGzDecoder;

use std::cmp::{max, min};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::BufReader;
use std::str;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct PrimerSet {
    name: String,
    primer: String,
    forward: bool,
    length: usize,
    index: u32,
    reference: String,
}

fn printrec(r: &Record, pname: &str, start: usize, end: usize) {
    let desc = format!("{}:{}", pname, r.desc().unwrap());
    print!(
        "{}",
        Record::with_attrs(
            r.id(),
            Some(&desc),
            &r.seq()[start..end],
            &r.qual()[start..end]
        )
    );
}

fn main() {
    let args = App::new("extract-amplicons")
        .version("0.1.0")
        .arg(
            Arg::with_name("primers")
                .index(1)
                .help("csv file for primer set"),
        )
        .arg(Arg::with_name("R1").index(2).help("first reads in pairs"))
        .arg(
            Arg::with_name("R2")
                .index(3)
                .help("second reads in pairs (reversed)"),
        )
        .arg(
            Arg::with_name("invert")
                .short("n")
                .required(false)
                .takes_value(false)
                .help("invert selection (show unmatching reads)"),
        )
        .arg(
            Arg::with_name("stats")
                .short("s")
                .required(false)
                .takes_value(false)
                .help("print stats about primer performance"),
        )
        .arg(
            Arg::with_name("trim")
                .short("t")
                .required(false)
                .takes_value(true)
                .help("trim bases from 3' end"),
        )
        .arg(
            Arg::with_name("ex")
                .short("x")
                .required(false)
                .takes_value(false)
                .help("excise primer sequence from reads"),
        )
        .get_matches();

    let mut primers = HashMap::new();
    let mut off_target: HashMap<String, u32> = HashMap::new();
    let mut on_target: HashMap<String, u32> = HashMap::new();
    let mut full_match = HashMap::new();

    let mut readbins = HashMap::new();
    let mut seqs = HashMap::new();
    let mut plen = 100;
    let mut pmax = 0;

    let trim: usize = match args.value_of("trim") {
        Some(s) => s.parse::<usize>().unwrap_or(0),
        None => 0,
    };

    for result in csv::Reader::from_reader(File::open(args.value_of("primers").unwrap()).unwrap())
        .deserialize()
    {
        let record: PrimerSet = result.unwrap();
        plen = min(record.length, plen);
        pmax = max(record.length, pmax);
    }

    for result in csv::Reader::from_reader(File::open(args.value_of("primers").unwrap()).unwrap())
        .deserialize()
    {
        let record: PrimerSet = result.unwrap();
        let primer = String::from(&record.primer[..plen]);
        let name = String::from(&record.name);
        primers.insert(
            primer,
            (String::from(&record.name), record.forward, record.length),
        );
        readbins.insert(name, BTreeMap::new());
        seqs.insert(String::from(&record.name), String::from(&record.reference));
    }

    let fq1 = Reader::new(MultiGzDecoder::new(BufReader::new(
        File::open(args.value_of("R1").unwrap()).unwrap(),
    )))
    .records();
    let fq2 = Reader::new(MultiGzDecoder::new(BufReader::new(
        File::open(args.value_of("R2").unwrap()).unwrap(),
    )))
    .records();

    let mut total_pairs = 0;
    let mut matched = 0;
    let grep = !(args.is_present("stats"));
    let invert = args.is_present("invert");
    let excise = args.is_present("ex");

    for (record1, record2) in fq1.zip(fq2) {
        match (record1, record2) {
            (Ok(r1), Ok(r2)) => {
                if r1.seq().len() > (plen + 80) && r2.seq().len() > (plen + 80) {
                    total_pairs += 1;
                    let p1 = String::from_utf8(r1.seq()[..plen].to_vec()).unwrap();
                    let p2 = String::from_utf8(r2.seq()[..plen].to_vec()).unwrap();
                    let p1end = r1.seq().len() - trim;
                    let p2end = r2.seq().len() - trim;

                    match (primers.get(&p1), primers.get(&p2)) {
                        (Some((p1name, _, p1len)), Some((p2name, _, p2len))) => {
                            if !(invert) & grep {
                                printrec(&r1, p1name, if excise { *p1len } else { 0 }, p1end);
                                printrec(&r2, p2name, if excise { *p2len } else { 0 }, p2end);
                            };

                            let seq1 = String::from_utf8(r1.seq()[*p1len..p1end].to_vec()).unwrap();
                            *(readbins.get_mut(&String::from(p1name)).unwrap())
                                .entry(seq1)
                                .or_insert(0) += 1;

                            let seq2 = String::from_utf8(r2.seq()[*p2len..p2end].to_vec()).unwrap();
                            *(readbins.get_mut(&String::from(p2name)).unwrap())
                                .entry(seq2)
                                .or_insert(0) += 1;

                            *full_match
                                .entry(format!("{}:{}", p1name, p2name))
                                .or_insert(0) += 1;
                            *on_target.entry(p1name.to_string()).or_insert(0) += 1;
                            *on_target.entry(p2name.to_string()).or_insert(0) += 1;
                            matched += 1
                        }
                        (Some((p1name, _, p1len)), None) => {
                            *off_target.entry(p2).or_insert(0) += 1;
                            *on_target.entry(p1name.to_string()).or_insert(0) += 1;
                            if invert & grep {
                                //printrec(&r1, p1name, *p1len);
                                print!("{}", r2);
                            }
                        }
                        (None, Some((p2name, _, p2len))) => {
                            *off_target.entry(p1).or_insert(0) += 1;
                            *on_target.entry(p2name.to_string()).or_insert(0) += 1;
                            if invert & grep {
                                print!("{}", r1);
                                //printrec(&r2, p2name, *p2len);
                            }
                        }
                        (None, None) => {
                            *off_target.entry(p1).or_insert(0) += 1;
                            *off_target.entry(p2).or_insert(0) += 1;

                            if invert & grep {
                                print!("{}", r1);
                                print!("{}", r2);
                            }
                        }
                    }
                }
            }
            _ => eprintln!("Not proper fastq file pair"),
        }
    }
    if !grep {
        //  for (primer, count) in on_target {
        //     if count > 1000 {
        //        println!("{}\t{}", primer, count);
        //     }
        //  }
        // for (primer, count) in off_target {
        //     if count > 1000 {
        //        println!("{}\t{}", primer, count);
        //     }
        // }
        let score = |a: u8, b: u8| if a == b { 1i32 } else { -1i32 };

        for (primer, reads) in readbins {
            print!("\n{}\t{}\t", args.value_of("R1").unwrap(), primer);
            let mut vars = 1;
            for (read, count) in reads {
                if count > 500 {
                    //                    println!("\t{}", seqs.get(&primer).unwrap());
                    let r = seqs.get(&primer).unwrap().as_bytes();
                    let x = read.as_bytes();
                    let mut aligner = Aligner::with_capacity(x.len(), r.len(), -3, -1, &score);
                    let alignment = aligner.semiglobal(x, r);
                    print!(
                        "\tallele:{},depth:{},score:{},",
                        vars, count, alignment.score
                    );
                    vars += 1;

                    if alignment.score < x.len() as i32 {
                        let mut i = 0;
                        for op in &alignment.operations {
                            match op {
                                Match => i += 1,
                                _ => print!("{}:{:?};", i, op),
                            }
                        }
                    }
                    print!("\t");
                    //if alignment.score < (x.len() as i32) - 6 {
                    //    print!("{}", alignment.pretty(x, r));
                    // }
                }
            }
        }
    }
    eprintln!(
        "{}\t{}/{}\tread pairs matched.",
        args.value_of("R1").unwrap(),
        matched,
        total_pairs
    );
}
