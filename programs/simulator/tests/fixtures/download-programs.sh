#!/usr/bin/env bash

for filename in programs/simulator/tests/fixtures/*.so; do
    filename=$(basename $filename)
    filename="${filename%.*}"
    echo $filename
    solana program dump -um $filename $SOURCE/$filename.so
done