"""Build the same locked Linux image when the Docker network cannot reach crates.io.
Vendored sources stay under ignored target/ and are supplied as a named build context.
"""
from pathlib import Path
import argparse
import subprocess
import tempfile
ROOT=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser()
parser.add_argument('--tag',default='nexofolio-backend:foundation')
parser.add_argument('--rust-image',default='rust:1.94.0-bookworm')
parser.add_argument('--runtime-image',default='debian:bookworm-slim')
args=parser.parse_args()
vendor=ROOT/'target/docker-vendor'
subprocess.run(['cargo','vendor','--locked','--versioned-dirs',str(vendor)],cwd=ROOT,check=True,stdout=subprocess.DEVNULL)
source=(ROOT/'deploy/Dockerfile').read_text()
needle='RUN cargo build --locked --release -p nexofolio-backend --bins'
assert source.count(needle)==1
replacement='''COPY --from=crates-vendor / /vendor
ENV RUSTUP_TOOLCHAIN=1.94.0
RUN mkdir -p .cargo && printf '[source.crates-io]\\nreplace-with = "vendored-sources"\\n[source.vendored-sources]\\ndirectory = "/vendor"\\n' > .cargo/config.toml
RUN cargo build --locked --offline --release -p nexofolio-backend --bins'''
with tempfile.TemporaryDirectory(prefix='nexofolio-offline-build-') as directory:
 dockerfile=Path(directory)/'Dockerfile';dockerfile.write_text(source.replace(needle,replacement))
 subprocess.run(['docker','build','--build-context',f'crates-vendor={vendor}','--build-arg',f'RUST_IMAGE={args.rust_image}','--build-arg',f'RUNTIME_IMAGE={args.runtime_image}','-f',str(dockerfile),'-t',args.tag,str(ROOT)],cwd=ROOT,check=True)
