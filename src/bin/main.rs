use clap::Parser;
use config::{Config, File, FileFormat};
use rust_fractal::renderer::FractalRenderer;
use rust_fractal::util::RecolourExr;

#[derive(Parser)]
#[command(version, about)]
struct Opts {
    #[clap(short, long, help = "Sets the location file to use")]
    input: Option<String>,

    #[clap(short = 'o', long, help = "Sets the options file to use")]
    options: Option<String>,

    #[clap(short = 'p', long, help = "Sets the palette file to use")]
    palette: Option<String>,

    #[clap(
        short = 'c',
        long,
        help = "Colours the EXR files in the output directory"
    )]
    colour_exr: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let opts: Opts = Opts::parse();

    let mut builder = Config::builder();

    if let Some(o) = opts.options {
        builder = builder.add_source(File::new(&o, FileFormat::Toml).required(true));
    };

    if let Some(p) = opts.palette {
        builder = builder.add_source(File::new(&p, FileFormat::Toml).required(true));
    };

    if let Some(l) = opts.input {
        builder = builder.add_source(File::with_name(&l).required(true));
    };

    let settings = builder.build()?;

    if opts.colour_exr {
        let colouring = RecolourExr::new(settings);
        colouring.colour();
    } else {
        let mut renderer = FractalRenderer::new(settings);
        renderer.render();
    }

    Ok(())

}
