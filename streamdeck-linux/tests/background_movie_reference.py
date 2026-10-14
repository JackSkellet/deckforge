#!/usr/bin/env python3
"""Compare the Rust background decoder with installed Qt on an owned WebP fixture."""
import os,shlex,subprocess
from pathlib import Path
project=Path(__file__).resolve().parents[1];output=project/'target/galleon/webp-reference';output.mkdir(parents=True,exist_ok=True)
flags=shlex.split(subprocess.check_output(['pkg-config','--cflags','--libs','Qt6Gui'],text=True));reader=output/'qt-reader'
subprocess.run(['c++','-std=c++17',str(project/'tests/qt_background_reference.cpp'),'-o',str(reader),*flags],check=True)
env=os.environ.copy();env.update(DECKFORGE_WEBP_REFERENCE_READER=str(reader),DECKFORGE_WEBP_REFERENCE_DIR=str(output),QT_QPA_PLATFORM='offscreen')
subprocess.run(['cargo','test','--locked','qt_webp_reference','--','--ignored','--nocapture'],cwd=project,env=env,check=True)
