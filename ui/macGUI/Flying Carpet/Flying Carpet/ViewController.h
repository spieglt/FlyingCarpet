//
//  ViewController.h
//  Flying Carpet
//
//  Created by Theron Spiegl on 8/26/17.
//  Copyright © 2017 Theron Spiegl. All rights reserved.
//

#import <Cocoa/Cocoa.h>

@interface ViewController : NSViewController

@property (weak) IBOutlet NSButton *Start;

@property (weak) IBOutlet NSButton *macOS;
@property (weak) IBOutlet NSButton *Windows;

@property (weak) IBOutlet NSButton *Sending;
@property (weak) IBOutlet NSButton *Receiving;

@property (weak) IBOutlet NSTextField *TextField;
@property (weak) IBOutlet NSTextField *InputField;

@property (weak) IBOutlet NSButton *ReceiveButton;
@property (weak) IBOutlet NSButton *SendButton;


@end

