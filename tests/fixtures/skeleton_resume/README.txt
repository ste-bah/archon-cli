Recorded skeleton-resume fixture from the R18 run.

The result records and stage-record stores are copied from the recorded run;
JSON whitespace is compacted where needed to keep files below 500 lines.
mtimes.json records original contract modification times. The test copies the
fixture to a temporary directory and restores those times with File::set_modified.
The event log is split into events-0.jsonl and events-1.jsonl, in original order.
No test writes to the source run or this fixture.
