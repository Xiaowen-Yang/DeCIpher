#!/bin/bash
set -e
echo "Running HPL benchmark..."
mpirun -np 4 xhpl
