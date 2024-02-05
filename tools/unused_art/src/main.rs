use fo_data::{FoRegistry, Retriever};
use fo_proto_format::ProtoItem;
use fo_unused_art::UnusedArtError;

pub fn main() {
    let retriever = FoRegistry::init("../../../client_tlj")
        .unwrap()
        .into_retriever();
    let protos = fo_proto_format::build_btree::<ProtoItem>("../../../FO4RP/proto/items/items.lst");
    let res = fo_unused_art::UnusedArt::prepare(protos.values()).find(&retriever);

    for file in &res.files {
        println!(
            "Unused {:?} in {:?}",
            file.conventional_path(),
            file.location(retriever.registry())
        );
    }

    for UnusedArtError {
        conventional_path,
        file_location,
        error,
    } in &res.errors
    {
        eprint!("[{conventional_path:?}] from {file_location:?}: {error:#?}");
        if let Ok(file) = retriever.file_by_path(conventional_path) {
            if let Ok(content) = std::str::from_utf8(&file) {
                eprintln!("\n===<CONTENT>===\n{content}\n===</CONTENT>===\n",);
            } else {
                eprintln!("; Can't show non-utf8 content!");
            }
        } else {
            eprintln!("; Can't read content!");
        }
    }
    eprintln!("{} unused files, total size: {}", res.files.len(), res.size);
}
