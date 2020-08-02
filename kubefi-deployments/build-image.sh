#!/usr/bin/env bash
# build musl binary via docker
rm -r target
alias rust-musl-builder='docker run --rm -it  -v "$(pwd)":/home/rust/src ekidd/rust-musl-builder:stable'
rust-musl-builder cargo build --release

# prepare content for the app image
mkdir target/docker
cp -r templates/ target/docker/templates
cp docker/Dockerfile target/docker
cp target/x86_64-unknown-linux-musl/release/kubefi-deployments target/docker/
mkdir -p target/docker/conf/
cp -r conf/ target/docker/conf

# build the app image
ver=$1
echo "version: $ver"
docker build -t kubefi-deployments-operator:${ver} -f target/docker/Dockerfile target/docker/