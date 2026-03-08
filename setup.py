from setuptools import setup, find_packages

setup(
    name="mimick",
    version="1.0.2",
    packages=find_packages(),
    include_package_data=True,
    package_data={
        'src': ['assets/*'],
    },
    install_requires=[
        "watchdog>=4.0.0",
        "requests>=2.31.0",
        "keyring>=24.3.0",
        "pystray>=0.19.5",
        "Pillow>=10.2.0",
        "PyGObject>=3.42.0",
    ],
    entry_points={
        "console_scripts": [
            "mimick=src.main:main",
        ],
    },
    author="Nick",
    author_email="nick@nickcardoso.com",
    description="A background daemon to auto-sync photos to Immich on Linux",
    long_description=open("README.md").read(),
    long_description_content_type="text/markdown",
    url="https://github.com/nicx17/immich_sync_app",
    classifiers=[
        "Development Status :: 4 - Beta",
        "Programming Language :: Python :: 3",
        "License :: OSI Approved :: GNU General Public License v3 (GPLv3)",
        "Operating System :: POSIX :: Linux",
    ],
    python_requires=">=3.10",
)
