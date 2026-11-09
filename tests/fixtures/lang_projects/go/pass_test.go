package proof

import "testing"

func TestPassing(t *testing.T) { if Add(1, 1) != 2 { t.Fatal("addition") } }
