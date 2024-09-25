use anyhow::Result;
use vergen_gitcl::{Emitter, GitclBuilder};

pub fn main() -> Result<()> {
    Emitter::default()
        .add_instructions(
            &GitclBuilder::default()
                .commit_date(true)
                .sha(true)
                .dirty(false)
                .build()?,
        )?
        .emit()
}
