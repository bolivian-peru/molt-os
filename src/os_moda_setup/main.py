import subprocess
import sys

# Function to install required dependencies
def install_dependencies():
    dependencies = [
        'nix',
        'rust',
        'cargo',
        'git'
    ]
    for dep in dependencies:
        print(f'Installing {dep}...')
        subprocess.run([sys.executable, '-m', 'pip', 'install', dep])

# Function to configure os-moda environment
def setup_os_moda():
    try:
        print('Setting up os-moda environment...')
        # Clone the repository
        subprocess.run(['git', 'clone', 'https://github.com/bolivian-peru/os-moda.git'], check=True)
        # Navigate to the project directory
        subprocess.run(['cd', 'os-moda'], check=True)
        # Build the project
        subprocess.run(['cargo', 'build'], check=True)
        # Deploy os-moda
        subprocess.run(['cargo', 'run'], check=True)
        print('os-moda setup completed successfully!')
    except subprocess.CalledProcessError as e:
        print(f'Error occurred: {e}')

if __name__ == '__main__':
    install_dependencies()
    setup_os_moda()