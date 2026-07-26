//
//  SsidBox.swift
//  FlyingCarpet
//
//  Created by Theron on 8/26/24.
//

import Cocoa

class SsidBox: NSTextField {
    override func becomeFirstResponder() -> Bool {
        super.becomeFirstResponder()
        // print("in focus")
        if self.stringValue == "" {
            self.stringValue = "AndroidShare_"
        }
        return true
    }
}
