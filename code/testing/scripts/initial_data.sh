#!/bin/bash
#Run this from the testing folder it builds the initial file structure
declare -a Dirs=("SP1" "SP2")
mkdir testground
COUNTER=0
for val in ${Dirs[@]}; do
     mkdir testground/$val
     mkdir testground/$val/peer1
     mkdir testground/$val/peer1/Owned
     mkdir testground/$val/peer1/Downloaded
     for i in {1..10};do
          head -c ${i}k </dev/urandom > "testground/$val/peer1/Owned/test${COUNTER}"
          COUNTER=$((COUNTER+1))
     done
done
echo "Hello, World! This file is used for the demo." > testground/SP1/peer1/Owned/testfile
