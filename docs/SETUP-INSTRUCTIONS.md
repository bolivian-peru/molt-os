# os-moda Setup Instructions

This guide will walk you through setting up the os-moda instance.

## Prerequisites
- Python 3.7 or later
- NixOS
- Rust

## Step 1: Clone the Repository
First, clone the repository from GitHub:

```bash
git clone https://github.com/bolivian-peru/os-moda.git
```

## Step 2: Install Dependencies
You will need to install a few dependencies before setting up the environment:

```bash
pip install nix rust cargo git
```

## Step 3: Build the Project
Navigate to the cloned directory and build the project:

```bash
cd os-moda
cargo build
```

## Step 4: Run os-moda
Run the following command to deploy the os-moda instance:

```bash
cargo run
```

Once this is done, os-moda will be up and running on your server.

## Video Tutorial
For a step-by-step video tutorial, please visit the official YouTube channel.