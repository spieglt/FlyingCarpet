package core

import (
	"testing"
)

func TestChopPaths(t *testing.T) {
	paths := []string{
		"/home/user/Desktop/a",
		"/home/user/Desktop/a/b/c/",
		"/home/user/Desktop/a/b/c",
		"/home/user/Desktop",
		"/home/user/Desktop/",
		"/home/user/Desktop/a/b/d/ce/f",
	}
	_, err := chopPaths(paths...)
	if err != nil {
		t.Error(err)
	}

	paths = append(paths, "/home/user/D")
	res, err := chopPaths(paths...)
	if err != nil { // doesn't fail because it's absolute so .. relative path can be determined
		t.Error(err)
	}
	if len(res) > 6 {
		t.Error("didn't strip relative path")
	}

	paths = append(paths, "user/D")
	_, err = chopPaths(paths...)
	if err == nil {
		t.Error("didn't return error for indeterminate path")
	}
}
