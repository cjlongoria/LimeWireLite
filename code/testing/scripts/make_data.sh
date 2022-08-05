#!/bin/bash
#Run this from the testing folder it will simulate random downloads

for x in {0..10};do
for i in {10..29};do
echo "search test${i}" >> input
done
done
echo "exit" >> input

