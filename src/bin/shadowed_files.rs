use fo_data::{
    crawler::{shadowed_files, ShadowedFile},
    datafiles::parse_datafile,
};

fn main() {
    let path = std::path::Path::new("../client_tlj")
        .canonicalize()
        .unwrap();
    let archives = parse_datafile(&path).expect("Parse datafiles");
    let files = shadowed_files(&archives).expect("Find shadowed files");
    let mut total_size = 0;
    for ShadowedFile {
        name,
        size,
        first_source: old,
        second_source: new,
    } in files
    {
        if old == new {
            continue;
        }
        println!(
            "File {:?} from {:?} replaced in {:?}",
            name,
            old.strip_prefix(&path).expect("strip prefix"),
            new.strip_prefix(&path).expect("strip prefix"),
        );
        total_size += size;
    }
    println!("Total shadowed size: {}", total_size);
}
