//
//  ViewController.m
//  Flying Carpet
//
//  Created by Theron Spiegl on 8/26/17.
//  Copyright © 2017 Theron Spiegl. All rights reserved.
//

#import "ViewController.h"
#include <sys/types.h>
#include <sys/stat.h>

@implementation ViewController

NSString *OS;
BOOL macSelected;
BOOL windowsSelected;
NSOpenPanel *sendFileSelector;
NSOpenPanel *receiveFolderSelector;

- (void)viewDidLoad {
    [super viewDidLoad];
    
    self.ReceiveButton.hidden = YES;
    self.SendButton.hidden = YES;
    self.InputField.hidden = YES;
    sendFileSelector = [NSOpenPanel openPanel];
    [sendFileSelector setCanChooseFiles:YES];
    [sendFileSelector setCanChooseDirectories:NO];
    [sendFileSelector setAllowsMultipleSelection:NO];
    
    receiveFolderSelector = [NSOpenPanel openPanel];
    [receiveFolderSelector setCanChooseFiles:NO];
    [receiveFolderSelector setCanChooseDirectories:YES];
    [receiveFolderSelector setAllowsMultipleSelection:NO];
    // Do any additional setup after loading the view.
}


- (void)setRepresentedObject:(id)representedObject {
    [super setRepresentedObject:representedObject];
    
    // Update the view, if already loaded.
}

- (IBAction)modeSelector:(id)sender {
    if ([self.Sending state] == NSOnState) {
        self.SendButton.hidden = NO;
        self.ReceiveButton.hidden = YES;
        self.InputField.hidden = NO;
    }
    if ([self.Receiving state] == NSOnState) {
        self.SendButton.hidden = YES;
        self.ReceiveButton.hidden = NO;
        self.InputField.hidden = NO;
    }
}

- (IBAction)osSelector:(id)sender {
    macSelected = ([self.macOS state] == NSOnState);
    windowsSelected = ([self.Windows state] == NSOnState);
}
- (IBAction)selectReceiveFile:(id)sender {
    [receiveFolderSelector runModal];
    self.InputField.stringValue = [NSString stringWithFormat:@"%@/outFile.extension",[receiveFolderSelector.URL path]];
}
- (IBAction)selectSendFile:(id)sender {
    [sendFileSelector runModal];
    self.InputField.stringValue = [NSString stringWithFormat:@"%@",[sendFileSelector.URL path]];
}

- (IBAction)StartButton:(id)sender {
    
    //    check that OS is selected
    if (macSelected) {
        self.TextField.stringValue = @"macOS";
    } else if (windowsSelected) {
        self.TextField.stringValue = @"Windows";
    } else {
        self.TextField.stringValue = @"Please select the OS of the other computer";
        return;
    }
    
    //    check that mode is selected, append to OS
    if ([self.Sending state] == NSOnState) {
        self.TextField.stringValue = [NSString stringWithFormat:@"%@\nSending",self.TextField.stringValue];
    } else if ([self.Receiving state] == NSOnState) {
        self.TextField.stringValue = [NSString stringWithFormat:@"%@\nReceiving",self.TextField.stringValue];
    } else {
        self.TextField.stringValue = [NSString stringWithFormat:@"Please select sending or receiving"];
        return;
    }
    
    //    check that file path is entered
    if (self.InputField.stringValue.length == 0) {
        self.TextField.stringValue = @"Please enter path of file to send or receive.";
        return;
    }
    
    //    check that file path isn't too long
    if ([self.InputField.stringValue length] > 1000) {
        self.TextField.stringValue = @"File path too long, 1000 characters max";
        return;
    }
    
    //    append and display the values
    self.TextField.stringValue = [NSString stringWithFormat:@"%@\n%@\n",self.TextField.stringValue, self.InputField.stringValue];
    
    
    //    convert to C string and send along named pipe
    const char *msg = self.TextField.stringValue.UTF8String;
    int res = mkfifo("/tmp/fc.fifo", 0666);
    if (res != 0) {
        printf("error making receiving fifo: %s\n", strerror(errno));
    } else {
        printf("fifo made\n");
    }
    
    int fd = open("/tmp/fc.fifo", O_WRONLY);
    if (fd < 0) {
        printf("error opening fifo: %s\n", strerror(errno));
    } else {
        printf("fifo opened\n");
    }
    NSLog(@"Connection Made");
    ssize_t n = write(fd, msg, strlen(msg));
    if (n <= 0) {
        printf("error writing message on fifo: %s\n", strerror(errno));
    }
    NSLog(@"Data written");
    close(fd);
    
    //    open window to capture output from Go?
}

@end
