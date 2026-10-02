mod chat;
mod config;
mod hub;
mod markdown;
mod provider;

use std::env;

use topcoat::{
    Result,
    asset::{AssetBundle, RouterBuilderAssetExt},
    router::{RouterBuilderDiscoverExt, Slot, layout, module_router},
    runtime::RouterBuilderRuntimeExt,
    view::{View, view},
};

use crate::{
    config::{Config, ProviderKind},
    hub::ChatHub,
    provider::Backend,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let config = Config::from_env()?;
    announce(&config);

    if config.provider == ProviderKind::InProcess {
        eprintln!("tc_chat: loading {model}…", model = config.model);
        #[cfg(not(feature = "metal"))]
        eprintln!(
            "tc_chat: in-process inference is on CPU. On Apple Silicon, re-run with --features metal."
        );
    }

    let backend = Backend::connect(&config).await?;
    let hub = ChatHub::new(backend, config.system_prompt);

    let router = module_router!()
        .assets(AssetBundle::load()?)
        .app_context(hub)
        .discover()
        .runtime()
        .build();

    topcoat::start(router).await?;
    Ok(())
}

fn announce(config: &Config) {
    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = env::var("PORT").unwrap_or_else(|_| "3000".into());
    match config.provider {
        ProviderKind::Ollama => eprintln!(
            "tc_chat: ollama  model={}  base={}  http://{host}:{port}",
            config.model, config.provider_base
        ),
        ProviderKind::InProcess => {
            eprintln!(
                "tc_chat: inprocess  model={}  http://{host}:{port}",
                config.model
            );
        }
    }
}

#[layout]
async fn layout(slot: Slot<'_>) -> Result<impl View> {
    Ok(view! {
        <!DOCTYPE html>
        <html lang="en" class="h-full">
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1">
                <title>"Chat"</title>
                topcoat::dev::script()
                topcoat::runtime::script()
                <link rel="stylesheet" href=(topcoat::tailwind::stylesheet!())>
            </head>
            <body class="h-dvh overflow-hidden bg-zinc-900 font-sans text-white antialiased">
                <main class="mx-auto flex h-full w-full max-w-3xl flex-col px-4 sm:px-6">
                    (slot)
                </main>
            </body>
        </html>
    })
}
