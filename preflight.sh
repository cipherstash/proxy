#!/bin/bash

set -e

MISSING=0

check_mise () {
  set +e
  mise_path="$(which mise)"
  retval=$?
  set -e
  if [ $retval -ne 0 ]; then
    MISSING=$(expr $MISSING + 1)
    echo "❌ mise is not installed"
    echo
    echo "   Follow the install instructions at https://github.com/jdx/mise#quickstart"
    echo
  else
    echo "✅ mise is installed"
  fi
}

check_docker () {
  set +e
  docker_path="$(which docker)"
  retval=$?
  set -e
  if [ $retval -ne 0 ]; then
    MISSING=$(expr $MISSING + 1)
    echo "❌ docker is not installed"
    echo
    echo "   Follow the install instructions for your platform: "
    echo
    echo "    - macOS: https://docs.docker.com/desktop/setup/install/mac-install/"
    echo "    - Linux: https://docs.docker.com/desktop/setup/install/linux/"
    echo
  else
    echo "✅ docker is installed"
  fi
}

check_rust () {
  set +e
  rustup_path="$(which rustup)"
  retval=$?
  set -e
  if [ $retval -ne 0 ]; then
    MISSING=$(expr $MISSING + 1)
    echo "❌ rust is not installed"
    echo
    echo "   Install with: "
    echo
    echo "   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    echo
  else
    echo "✅ rust is installed"
  fi
}

check_cargo_binstall () {
  set +e
  cargo_binstall_path="$(which cargo-binstall)"
  retval=$?
  set -e
  if [ $retval -ne 0 ]; then
    MISSING=$(expr $MISSING + 1)
    echo "❌ cargo-binstall is not installed"
    echo
    echo "   Install with: "
    echo
    echo "   curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash"
    echo
  else
    echo "✅ cargo-binstall is installed"
  fi
}

main () {
  echo "Checking development dependencies are present:\n"
  check_mise
  check_docker
  check_rust
  check_cargo_binstall
  echo
  if [ $MISSING -gt 0 ]; then
    echo "❌ Looks like you have ${MISSING} missing dependencies."
    echo
    echo "Follow the install instructions above, and re-run this script to re-check."
  else
    echo "Looks like you are good to go!"
  fi
  echo
}

main
