use git2::Repository;
fn main() {
    let _repo = Repository::discover(".").unwrap();
    // we want to find out the type of commit.summary()
}
