package proof

import "testing"

func TestFailing(t *testing.T) { if Add(1, 1) != 3 { t.Fatal("addition") } }
