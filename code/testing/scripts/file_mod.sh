#!/bin/bash
#Run this from the testing folder it simulates file changes
while true; do
    ls testground|sort -R|tail -1|while read dir; do
        ls testground/$dir/peer1/Owned|sort -R| tail -1| while read file; do
            echo $file
            echo "foo" >> testground/$dir/peer1/Owned/$file
        done
    done
    sleep 1;
done
