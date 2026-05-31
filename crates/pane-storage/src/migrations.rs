refinery::embed_migrations!("migrations");

pub fn runner() -> refinery::Runner {
    migrations::runner()
}
