mod cli;

use anyhow::Result;
use clap::Parser;

use cli::args::{Cli, Commands};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.dev_help {
        cli::util::print_developer_help();
        return Ok(());
    }

    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            use clap::CommandFactory;
            Cli::command().print_help()?;
            println!();
            return Ok(());
        }
    };

    match command {
        Commands::DecryptModel { source, dest } => cli::decrypt::run(source, dest).await?,
        Commands::Detect {
            input,
            output,
            model,
            json,
            jobs,
            translate_free_text,
            ocr_script,
            ocr,
            ocr_model,
            format,
            quiet,
        } => {
            cli::detect::run(
                input,
                output,
                model,
                json,
                jobs,
                translate_free_text,
                ocr_script,
                ocr,
                ocr_model,
                format,
                quiet,
            )
            .await?
        }
        Commands::Inpaint {
            input,
            output,
            model,
        } => cli::inpaint::run(input, output, model).await?,
        Commands::Translate {
            input,
            output,
            export,
            model,
            target_lang,
            prompt,
            batch_size,
            provider,
            fallback_provider,
            rate_limit,
            jobs,
            no_cache,
            clear_cache,
            gemini_key,
            openai_key,
            openai_base_url,
            openai_model,
            gemini_model,
            claude_key,
            claude_model,
            font,
            cjk_font,
            save_metadata,
            metadata_dir,
            ocr,
            translate_free_text,
            ocr_script,
            mode,
            ocr_model,
            glossary,
            progress,
            format,
            quiet,
            retry_failed,
        } => {
            cli::translate::run(
                input,
                output,
                export,
                model,
                target_lang,
                prompt,
                batch_size,
                provider,
                fallback_provider,
                rate_limit,
                jobs,
                no_cache,
                clear_cache,
                gemini_key,
                openai_key,
                openai_base_url,
                openai_model,
                gemini_model,
                claude_key,
                claude_model,
                font,
                cjk_font,
                save_metadata,
                metadata_dir,
                ocr,
                translate_free_text,
                ocr_script,
                mode,
                ocr_model,
                glossary,
                progress,
                format,
                quiet,
                retry_failed,
            )
            .await?
        }
        Commands::Metadata { cmd } => cli::metadata::run(cmd).await?,
        Commands::Font { cmd } => cli::font::run(cmd).await?,
    }
    Ok(())
}
