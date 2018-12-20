package main

import (
	"flag"
	"fmt"
	"log"
	"os"

	"github.com/spieglt/flyingcarpet/core"
)

func main() {
	// get flags
	if len(os.Args) == 1 {
		printUsage()
		return
	}

	var pOutFile = flag.String("send", "", "File to be sent. (Use [ -send multi ] for multiple files, and list files/globs after other flags.)\n\n"+
		"Example (Windows): .\\flyingcarpet.exe -send multi -peer mac pic1.jpg pic35.jpg \"filename with spaces.docx\" *.txt\n"+
		"Example (macOS/Linux): ./flyingcarpet -send multi -peer windows movie.mp4 ../*.mp3\n")
	var pInFolder = flag.String("receive", "", "Destination directory for files to be received.")
	var pPort = flag.Int("port", 3290, "TCP port to use (must match on both ends).")
	var pPeer = flag.String("peer", "", "Use \"-peer linux\", \"-peer mac\", or \"-peer windows\" to match the other computer.")
	flag.Parse()
	outFile := *pOutFile
	inFolder := *pInFolder
	port := *pPort
	peer := *pPeer

	// validate
	if peer == "" || (peer != "mac" && peer != "windows" && peer != "linux") {
		log.Fatal("Must choose [ -peer linux|mac|windows ].")
	}

	t := core.Transfer{}

}

func printUsage() {
	fmt.Println("\nSingle file usage:")
	fmt.Println("(Windows) $ flyingcarpet.exe -send ./movie.mp4 -peer mac")
	fmt.Println("[Enter password from receiving end.]")
	fmt.Println("  (Mac)   $ ./flyingcarpet -receive ./destinationFolder -peer windows")
	fmt.Println("[Enter password into sending end.]\n")

	fmt.Println("Multiple file usage:")
	fmt.Println(" (Linux)  $ ./flyingcarpet -send multi -peer windows ../Pictures/*.jpg \"Filename with spaces.txt\" movie.mp4")
	fmt.Println("[Enter password from receiving end.]")
	fmt.Println("(Windows) $ flyingcarpet.exe -receive .\\picturesFolder -peer linux")
	fmt.Println("[Enter password into sending end.]\n")
	return
}
